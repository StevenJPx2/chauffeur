import { request as httpRequest } from "node:http"
import { request as httpsRequest } from "node:https"

type Reply = { readonly status: number; readonly body: string }

/**
 * A dedicated socket, not the host's fetch pool: long-lived MCP and SourceFed
 * streams must not queue permission checks before they reach the daemon.
 * The caller's abort signal covers both response headers and the whole body.
 */
export function postRpc(url: string, headers: Headers, body: string, signal: AbortSignal): Promise<Reply> {
  return new Promise((resolve, reject) => {
    const target = new URL(url)
    const send = target.protocol === "https:" ? httpsRequest : httpRequest

    const request = send(target, {
      method: "POST",
      headers: Object.fromEntries(headers),
      agent: false,
      signal,
    }, (response) => {
      let received = ""

      response.setEncoding("utf8")
      response.on("data", (chunk: string) => { received += chunk })
      response.once("error", reject)
      response.once("end", () => resolve({ status: response.statusCode ?? 0, body: received }))
    })

    request.once("error", reject)
    request.end(body)
  })
}
