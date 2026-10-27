# `config`

Loads `models.json`: every registered model and each tier's models in
fallback order. Shipped files load first and the user's last, so the user's
entries win.

| File | Holds |
|---|---|
| `mod.rs` | `Models::load` over files in order, and the check that every tier names a registered model |
| `entry.rs` | `ModelEntry`, one variant per API format, and `KeySource`, the environment variable holding a key |
| `error.rs` | `ConfigError`: a file that cannot be read or parsed, a malformed placeholder answer, or an unknown model |

```json
{
  "models": {
    "jev": { "format": "systemone", "endpoint": "https://api.example", "model": "jev-1",
             "key": { "env": "TYPESAFE_API_KEY" } },
    "offline": { "format": "placeholder", "otherwise": { "yes": 0.5, "no": 0.5 } }
  },
  "tiers": { "system-one": ["jev", "offline"] }
}
```

```rust
use chauffeur_judges::{Models, Tier};

let models = Models::load(&[shipped, user])?;
assert_eq!(models.tier(Tier::SystemOne), ["jev", "offline"]);
```

- A later file's model replaces an earlier one of the same name; a later
  tier list replaces the earlier list.
- A placeholder's `answers` path is made relative to the file that names it.
- Each format's entry type lives with its format: `SystemOneEntry` in
  `tiers/system_one/`, `PlaceholderEntry` in `placeholder/`.
