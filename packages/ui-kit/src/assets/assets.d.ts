/**
 * Type declarations for the asset imports the ui-kit makes.
 *
 * Declared by hand rather than through `/// <reference types="vite/client" />`
 * because the ui-kit is a source package with no build step and no Vite
 * dependency of its own — it is compiled by `tsc -b` for checking and bundled
 * by whichever app consumes it. Referencing Vite's client types here would add
 * a dependency on a bundler the package does not otherwise know about, to
 * declare two module patterns we can spell out in eight lines.
 *
 * All three consuming apps are Vite-based, so `?raw` and `?url` resolve
 * identically in each. See `components/ConnectorIcon.tsx` for the only use.
 */

/** The file's text, inlined at build time. Used for brand SVGs, which are
 *  rendered inline so they can be sized by their container. */
declare module "*.svg?raw" {
  const content: string;
  export default content;
}

/** The file's emitted URL. */
declare module "*.svg?url" {
  const url: string;
  export default url;
}
