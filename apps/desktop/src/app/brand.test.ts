/**
 * The Evara brand asset, as shipped.
 *
 * Two copies of one mark exist and must stay identical: the desktop icon set under
 * `src-tauri/icons/`, which Windows and macOS installers use, and
 * `src/assets/evara-logo.png`, which the interface renders. `assets/README.md` explains
 * why there are two.
 *
 * Nothing in a bundler or a build notices if they drift, and the symptom would be an
 * application whose window shows one mark and whose taskbar shows another — the kind of
 * thing that ships and then takes a year to be reported. So it is a test.
 *
 * Read off disk through an indirect `node:fs` import: the frontend's `tsconfig`
 * deliberately has no Node types, and importing a PNG here would give a bundler URL rather
 * than the bytes.
 */
import { beforeAll, describe, expect, it } from "vitest";

/** The slice of `node:fs` this file needs. */
interface FsModule {
  readFileSync: (path: string) => Uint8Array;
  existsSync: (path: string) => boolean;
}

let fs: FsModule;

beforeAll(async () => {
  const specifier = "node:fs";
  fs = (await import(/* @vite-ignore */ specifier)) as unknown as FsModule;
});

/** Paths are relative to the Vite root, which is `apps/desktop`. */
const FRONTEND_LOGO = "src/assets/evara-logo.png";
const ICON_128 = "src-tauri/icons/128x128.png";

/** Every icon `tauri.conf.json` names, which is what an installer ships. */
const BUNDLED_ICONS = [
  "src-tauri/icons/32x32.png",
  "src-tauri/icons/128x128.png",
  "src-tauri/icons/128x128@2x.png",
  "src-tauri/icons/icon.icns",
  "src-tauri/icons/icon.ico",
];

describe("the desktop icon set", () => {
  it("has every icon the bundle configuration names", () => {
    // A missing icon fails `tauri build` rather than the frontend suite, which is a long
    // way from here and only on a release build.
    for (const path of BUNDLED_ICONS) {
      expect(fs.existsSync(path), `${path} is missing`).toBe(true);
    }
  });

  it("keeps the master artwork the icons are generated from", () => {
    // `npx tauri icon src-tauri/icons/source.png` regenerates the whole set from this.
    expect(fs.existsSync("src-tauri/icons/source.png")).toBe(true);
  });

  it("ships Windows and macOS icons in their platform formats", () => {
    // `.ico` and `.icns` are what the Windows installer and the macOS bundle need; a PNG
    // renamed to either would be accepted by the filesystem and rejected by the installer.
    const ico = fs.readFileSync("src-tauri/icons/icon.ico");
    expect([ico[0], ico[1], ico[2], ico[3]]).toEqual([0x00, 0x00, 0x01, 0x00]);

    const icns = fs.readFileSync("src-tauri/icons/icon.icns");
    expect(String.fromCharCode(icns[0]!, icns[1]!, icns[2]!, icns[3]!)).toBe("icns");
  });
});

describe("the frontend's copy of the mark", () => {
  it("is byte-identical to the 128px desktop icon", () => {
    // The whole reason this file exists. If someone redraws the logo they must regenerate
    // both, and this is what tells them they only did one.
    const frontend = fs.readFileSync(FRONTEND_LOGO);
    const icon = fs.readFileSync(ICON_128);

    expect(frontend.length).toBe(icon.length);
    expect(Array.from(frontend)).toEqual(Array.from(icon));
  });

  it("is a real PNG", () => {
    const bytes = fs.readFileSync(FRONTEND_LOGO);
    expect(Array.from(bytes.slice(0, 8))).toEqual([
      0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
    ]);
  });

  it("is small enough to inline without thought", () => {
    // It is imported into the bundle, so its size is the application's size. A logo that
    // grew to a megabyte would be a mistake worth catching here.
    expect(fs.readFileSync(FRONTEND_LOGO).length).toBeLessThan(64 * 1024);
  });
});
