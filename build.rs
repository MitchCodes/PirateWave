fn main() {
    println!("cargo:rerun-if-changed=assets/branding/piratewave.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/branding/piratewave.ico")
            .set("ProductName", "PirateWave")
            .set("FileDescription", "PirateWave Audio Workspace")
            .set("OriginalFilename", "PirateWave.exe")
            .compile()
            .expect("Compile PirateWave Windows icon and version resources");
    }
}
