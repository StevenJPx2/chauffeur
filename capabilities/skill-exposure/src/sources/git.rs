//! Git sources through one persistent partial clone per source. A change
//! check is one `ls-remote`; a change fetches trees only (`blob:none`) and
//! the `SKILL.md` blobs; a skill's other files are fetched when attached.

use std::path::Path;
use std::process::Stdio;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use super::{
    Check, ListedFile, ListedSkill, Listing, MAX_BYTES, MAX_FILE, MAX_FILES, Source, SourceError,
    Validator, parse, store, valid_path,
};

/// One source's partial clone.
pub(super) struct Repository<'a> {
    pub path: &'a Path,
}

impl Repository<'_> {
    /// Missing blobs are fetched from the promisor remote on demand.
    async fn run(&self, args: &[&str]) -> Result<Vec<u8>, SourceError> {
        self.run_with(args, None).await
    }

    /// Stdin and both pipes are driven together. Cancellation kills the direct
    /// child, and no shell is used. With `input`, lazy blob fetches are off:
    /// the only stdin callers are local object queries.
    async fn run_with(
        &self,
        args: &[&str],
        input: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, SourceError> {
        let mut command = Command::new("git");

        if input.is_some() {
            command.env("GIT_NO_LAZY_FETCH", "1");
        } else {
            command.env_remove("GIT_NO_LAZY_FETCH");
        }

        let mut child = command
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "protocol.file.allow=never",
                "-c",
                "protocol.ext.allow=never",
                "-c",
                "gc.auto=0",
            ])
            .arg("--git-dir")
            .arg(self.path)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env(
                "GIT_SSH_COMMAND",
                "ssh -o BatchMode=yes -o ConnectTimeout=5",
            )
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take();
        let feed = async move {
            if let (Some(mut stdin), Some(input)) = (stdin, input) {
                stdin.write_all(&input).await?;
            }

            Ok::<(), std::io::Error>(())
        };
        let stdout = child
            .stdout
            .take()
            .ok_or(SourceError::Invalid("Git stdout was not piped"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(SourceError::Invalid("Git stderr was not piped"))?;
        let mut output = Vec::new();
        let mut diagnostics = Vec::new();
        let limit = u64::try_from(MAX_BYTES.saturating_add(1))
            .map_err(|_| SourceError::Invalid("invalid output limit"))?;
        let mut stdout = stdout.take(limit);
        let mut stderr = stderr.take(16_385);
        let (fed, out, err, status) = tokio::join!(
            feed,
            stdout.read_to_end(&mut output),
            stderr.read_to_end(&mut diagnostics),
            child.wait()
        );

        fed?;
        out?;
        err?;
        let status = status?;

        if output.len() > MAX_BYTES {
            return Err(SourceError::Invalid("Git output exceeds its limit"));
        }

        if !status.success() {
            return Err(SourceError::Git(status));
        }

        Ok(output)
    }

    /// Fetch the listed blobs the partial clone lacks in one request, as Git's
    /// own promisor fetch does, instead of one round trip per file on first read.
    async fn prefetch(&self, objects: &[&str]) -> Result<(), SourceError> {
        let query: String = objects.iter().map(|object| format!("{object}\n")).collect();
        let answer = self
            .run_with(&["cat-file", "--batch-check"], Some(query.into_bytes()))
            .await?;
        let answer = std::str::from_utf8(&answer)
            .map_err(|_| SourceError::Invalid("invalid cat-file reply"))?;
        // One line per query, in order: `<oid> missing` for an absent object.
        let missing: Vec<&str> = objects
            .iter()
            .zip(answer.lines())
            .filter(|(_, line)| line.ends_with(" missing"))
            .map(|(object, _)| *object)
            .collect();

        if missing.is_empty() {
            return Ok(());
        }

        let mut args = vec![
            "-c",
            "fetch.negotiationAlgorithm=noop",
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--recurse-submodules=no",
            "--filter=blob:none",
            "origin",
        ];

        args.extend(missing);
        self.run(&args).await?;

        Ok(())
    }

    /// The contents of `objects`, in order, fetched first in one request when missing.
    async fn blobs(&self, objects: &[&str]) -> Result<Vec<Vec<u8>>, SourceError> {
        if objects.is_empty() {
            return Ok(Vec::new());
        }

        self.prefetch(objects).await?;
        let query: String = objects.iter().map(|object| format!("{object}\n")).collect();
        let output = self
            .run_with(&["cat-file", "--batch"], Some(query.into_bytes()))
            .await?;
        let mut rest = output.as_slice();
        let mut blobs = Vec::with_capacity(objects.len());

        // Each reply is `<oid> <type> <size>\n<size bytes>\n`.
        for object in objects {
            let end = rest
                .iter()
                .position(|byte| *byte == b'\n')
                .ok_or(SourceError::Invalid("truncated cat-file reply"))?;
            let header = std::str::from_utf8(rest.get(..end).unwrap_or_default())
                .map_err(|_| SourceError::Invalid("invalid cat-file header"))?;
            let mut fields = header.split(' ');
            let size: usize = match (fields.next(), fields.next(), fields.next()) {
                (Some(oid), Some("blob"), Some(size)) if oid == *object => size
                    .parse()
                    .map_err(|_| SourceError::Invalid("invalid cat-file size"))?,
                _ => {
                    return Err(SourceError::Invalid(
                        "listed Git object is not a fetched blob",
                    ));
                }
            };
            let start = end.saturating_add(1);
            let stop = start.saturating_add(size);
            let blob = rest
                .get(start..stop)
                .ok_or(SourceError::Invalid("truncated cat-file reply"))?;

            blobs.push(blob.to_vec());
            rest = rest.get(stop.saturating_add(1)..).unwrap_or_default();
        }

        Ok(blobs)
    }

    /// Create the bare clone once and point `origin` at the configured URL.
    async fn ensure(&self, url: &str) -> Result<(), SourceError> {
        if !self.path.join("HEAD").is_file() {
            let path = self
                .path
                .to_str()
                .ok_or(SourceError::Invalid("non-UTF-8 cache path"))?;

            self.run(&["init", "--bare", "--quiet", path]).await?;
        }

        self.run(&["config", "remote.origin.url", url]).await?;

        Ok(())
    }

    /// The commit `reference` names upstream: a pinned commit as is, else one `ls-remote`.
    async fn head(&self, reference: &str) -> Result<String, SourceError> {
        if is_commit(reference) {
            return Ok(reference.to_owned());
        }

        let listed = self.run(&["ls-remote", "--", "origin", reference]).await?;
        let listed =
            std::str::from_utf8(&listed).map_err(|_| SourceError::Invalid("invalid ls-remote"))?;
        let refs: Vec<(&str, &str)> = listed
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .collect();
        let wanted = [
            reference.to_owned(),
            format!("refs/heads/{reference}"),
            format!("refs/tags/{reference}^{{}}"),
            format!("refs/tags/{reference}"),
        ];

        wanted
            .iter()
            .find_map(|name| refs.iter().find(|(_, listed)| listed == name))
            .map(|(commit, _)| (*commit).to_owned())
            .filter(|commit| is_commit(commit))
            .ok_or(SourceError::Invalid("Git ref not found upstream"))
    }

    pub(super) async fn check(
        &self,
        source: &Source,
        current: Option<&Listing>,
        now: u64,
    ) -> Result<Check, SourceError> {
        let Source::Git {
            url,
            reference,
            paths,
            ..
        } = source
        else {
            return Err(SourceError::Invalid("not a Git source"));
        };

        self.ensure(url).await?;
        let head = self.head(reference).await?;

        if current.is_some_and(|listing| listing.revision == head) {
            return Ok(Check::Unchanged);
        }

        self.run(&[
            "fetch",
            "--quiet",
            "--filter=blob:none",
            "--depth=1",
            "--no-tags",
            "origin",
            reference,
        ])
        .await?;
        let fetched = String::from_utf8(self.run(&["rev-parse", "FETCH_HEAD"]).await?)
            .map_err(|_| SourceError::Invalid("invalid Git revision"))?
            .trim()
            .to_owned();

        // Keep each listed revision's trees reachable for later attaches.
        self.run(&["update-ref", &format!("refs/chauffeur/{fetched}"), &fetched])
            .await?;
        let skills = self.skills(&fetched, paths).await?;

        Ok(Check::Changed(Listing {
            source: source.clone(),
            revision: fetched,
            checked_at: now,
            validator: Validator::default(),
            skills,
        }))
    }

    /// Every `SKILL.md` under the selected roots, with its folder's regular files.
    async fn skills(
        &self,
        commit: &str,
        roots: &[String],
    ) -> Result<Vec<ListedSkill>, SourceError> {
        let tree = self.run(&["ls-tree", "-rz", "--full-tree", commit]).await?;
        let tree =
            std::str::from_utf8(&tree).map_err(|_| SourceError::Invalid("non-UTF-8 Git paths"))?;
        let entries: Vec<Entry<'_>> = tree.split('\0').filter_map(Entry::parse).collect();
        let mains: Vec<&Entry<'_>> = entries
            .iter()
            .filter(|entry| {
                (entry.path == "SKILL.md" || entry.path.ends_with("/SKILL.md"))
                    && roots.iter().any(|root| within(root, entry.path))
            })
            .collect();
        let mut skills = Vec::new();

        if mains.len() > MAX_FILES {
            return Err(SourceError::Invalid("a source has more than 512 skills"));
        }

        let texts = self
            .blobs(&mains.iter().map(|entry| entry.object).collect::<Vec<_>>())
            .await?;

        for (entry, text) in mains.into_iter().zip(texts) {
            let dir = entry.path.strip_suffix("SKILL.md").unwrap_or_default();
            let parsed = parse::skill(&text)?;
            let files: Vec<ListedFile> = entries
                .iter()
                .filter_map(|file| {
                    let path = file.path.strip_prefix(dir)?;

                    (path != "SKILL.md" && valid_path(path)).then(|| ListedFile {
                        path: path.to_owned(),
                        executable: file.executable,
                        object: Some(file.object.to_owned()),
                    })
                })
                .collect();

            if files.len() > MAX_FILES {
                return Err(SourceError::Invalid(
                    "a skill folder has more than 512 files",
                ));
            }

            skills.push(ListedSkill {
                id: parsed.id,
                description: parsed.description,
                body: parsed.body,
                dir: dir.trim_end_matches('/').to_owned(),
                files,
            });
        }

        Ok(skills)
    }

    /// Write one skill's files into `staging`, fetching the missing blobs in one request.
    pub(super) async fn materialize(
        &self,
        skill: &ListedSkill,
        staging: &Path,
    ) -> Result<(), SourceError> {
        let mut total = 0_usize;
        let objects = skill
            .files
            .iter()
            .map(|file| {
                file.object
                    .as_deref()
                    .filter(|object| is_commit(object))
                    .ok_or(SourceError::Invalid("listed Git file has no blob id"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let blobs = self.blobs(&objects).await?;

        for (file, bytes) in skill.files.iter().zip(blobs) {
            total = total.saturating_add(bytes.len());

            if bytes.len() > MAX_FILE || total > MAX_BYTES {
                return Err(SourceError::Invalid("skill files exceed their size limit"));
            }

            store::write_file(staging, &file.path, &bytes, file.executable).await?;
        }

        Ok(())
    }
}

/// A regular file in `ls-tree -rz` output; symlinks and submodules are skipped.
struct Entry<'a> {
    path: &'a str,
    object: &'a str,
    executable: bool,
}

impl<'a> Entry<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let (metadata, path) = line.split_once('\t')?;
        let mut fields = metadata.split(' ');
        let executable = match fields.next()? {
            "100644" => false,
            "100755" => true,
            _ => return None,
        };

        (fields.next()? == "blob").then_some(())?;
        let object = fields.next().filter(|object| is_commit(object))?;

        Some(Self {
            path,
            object,
            executable,
        })
    }
}

fn within(root: &str, path: &str) -> bool {
    root == "." || path.starts_with(&format!("{root}/"))
}

/// A full SHA-1 or SHA-256 object name.
fn is_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}
