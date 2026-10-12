//! Tauri's build script: generates context, embeds the config and the icons.

fn main() {
    // `tauri_build` compiles `icons/icon.ico` into the executable's Win32 resource, which
    // is what Windows shows in the title bar, the taskbar and Explorer. It does not
    // declare that file as a build input, so replacing the mark leaves a cached
    // `resource.lib` in place and the rebuilt binary keeps the previous icon -- a stale
    // taskbar icon that no amount of rebuilding clears, and nothing in the output hints
    // why. Naming the file here is what makes `cargo` re-run this script when it changes.
    //
    // Emitting a `rerun-if-changed` of our own narrows the default "any file in the
    // package" rule, so the other inputs are named too rather than inherited.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=capabilities");

    tauri_build::build();
}
