# Frontend assets

## `evara-logo.png`

The Evara application mark, as shown in the interface's branding areas and as the fallback
for a school that has not uploaded a logo of its own.

**It is a copy of `apps/desktop/src-tauri/icons/128x128.png`**, which is itself generated
from `src-tauri/icons/source.png` — the master artwork, and the one file to replace when
the mark changes.

`source.png` is the approved mark prepared for use as an icon: the artwork cropped to the
rounded square, squared, and alpha-cut along that silhouette so the corners are
transparent rather than white. The supplied artwork sat on a white canvas, which is
correct for a picture and wrong for an icon — a white-cornered square is what Windows
would have shown in the taskbar. Nothing inside the silhouette was altered; the pale
blue-white gradient and the calendar are the design as approved.

The copy exists because the frontend bundle cannot reach outside its own root, and Vite
resolving an import from `src-tauri/` would couple the web build to the Tauri crate's
layout. One small PNG is a cheaper price than that coupling.

**If the logo is ever finalised or redrawn, regenerate both from `source.png`** — the Tauri
icon set with `npx tauri icon`, and this file by copying `128x128.png` again. They are the
same mark and must not drift apart. `brand.test.ts` asserts the two files are byte-identical,
so drift is a failing test rather than something nobody notices.

`npx tauri icon` also writes `icons/android/` and `icons/ios/`. Evara ships neither, and
those directories are deleted rather than committed.

The bundled icon set is also what Windows and macOS installers use; see `tauri.conf.json`.
