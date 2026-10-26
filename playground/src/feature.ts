/** One thing the playground tests, shown as its own tab. */
export type Feature = {
  /** Used in the URL, as `#id`. */
  readonly id: string
  readonly title: string
  /** The tab's reactive scope; `$template` names its `<template>` in index.html. */
  readonly view: () => { readonly $template: string }
}
