import { defineWranglerConfig } from "wrangler/experimental-config"

// cf builds a static site with Wrangler, which reads the folder to upload here.
export default defineWranglerConfig({
  assetsDirectory: ".vitepress/dist",
})
