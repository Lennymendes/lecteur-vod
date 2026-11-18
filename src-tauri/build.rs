fn main() {
    // Triplet de la cible : sert à retrouver les sidecars `binaries/<nom>-<triplet>` en développement.
    println!("cargo:rustc-env=AUREX_TARGET={}", std::env::var("TARGET").unwrap_or_default());
    tauri_build::build()
}
