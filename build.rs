fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    #[cfg(target_os = "windows")]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icon.ico");
        resource.set("ProductName", "EmberClick");
        resource.set("FileDescription", "EmberClick Auto Clicker");
        resource.set("LegalCopyright", "Copyright © 2026 ArturoGon06");
        resource
            .compile()
            .expect("failed to embed Windows application resources");
    }
}
