import { defineConfig } from "cf/config"

export default defineConfig({
  accountId: "7bb7dff27822397d46162181bfedf38d",
  worker: {
    name: "chauffeur-docs",
    compatibilityDate: "2026-10-06",
    assets: { notFoundHandling: "404-page" },
  },
})
