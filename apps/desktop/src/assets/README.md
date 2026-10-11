# Frontend assets

## `evara-logo.png`

The Evara application mark, as shown in the interface's branding areas and as the fallback
for a school that has not uploaded a logo of its own.

**It is a copy of `apps/desktop/src-tauri/icons/128x128.png`**, which is itself generated
from `src-tauri/icons/source.png` — the master artwork, committed with the desktop
foundation. No new artwork was drawn for it.

The copy exists because the frontend bundle cannot reach outside its own root, and Vite
resolving an import from `src-tauri/` would couple the web build to the Tauri crate's
layout. One 3.7 KB PNG is a cheaper price than that coupling.

**If the logo is ever finalised or redrawn, regenerate both from `source.png`** — the Tauri
icon set with `npx tauri icon`, and this file by copying `128x128.png` again. They are the
same mark and must not drift apart. `brand.test.ts` asserts the two files are byte-identical,
so drift is a failing test rather than something nobody notices.

The bundled icon set is also what Windows and macOS installers use; see `tauri.conf.json`.
