# pacsleaf-viewer

React + TypeScript web viewer shipped with `pacsnode`.

It provides the pacsleaf study browser, client-side 2D/MPR/VR viewing, and an
optional streamed rendering mode backed by `pacsleaf-streamer`. When hosted by
`pacsnode`, this is the default bundled viewer at `/viewer`; OHIF remains
available separately at `/ohif`.

## Local development

```bash
npm install
npm run dev
```

Useful commands:

```bash
npm run build
npm run lint
```

The app expects runtime settings from `app-config.js` when served by the
`pacsleaf-viewer` plugin. A fallback config is baked in for local/frontend-only
development.

## pacsnode integration

`cargo build` in the `pacsnode` workspace automatically builds this frontend and
embeds the generated `dist/` bundle into `pacs-pacsleaf-viewer-plugin`.

If you intentionally want to skip the frontend build during a Rust build, set:

```bash
PACSNODE_SKIP_PACSLEAF_WEB_BUILD=1
```

The plugin build also repairs a cross-platform `node_modules` install by
running `npm ci` when the required Rolldown native binding is missing.

If your environment needs a non-default package manager command, set:

```bash
PACSNODE_PACSLEAF_NPM=npm
```

On Windows the build script now defaults to `npm.cmd`.
