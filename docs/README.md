# Argus project site

Served by GitHub Pages from `main` → `/docs` at
<https://cybercore-tech.github.io/argus/>.

- `index.html`: page content, plus `window.SITE` (default theme and signature mashups)
- `styles.css`, `app.js`: the shared Cybercore project-site kit (same files on every family site)

Themes load at runtime from the org site's registry (`/data/cybergrid.json`,
`/data/themes/<family>/<name>.json` on cybercore-tech.github.io), so every
project site shares one source of truth. A signature mashup takes surfaces
(bg/panel/line/muted/white) from its `base` palette and neons from its
`accent` palette.

Local preview: serve a folder containing `argus/` (this dir) and `data/`
(copied or symlinked from the org site repo), then open
`http://127.0.0.1:<port>/argus/`.
