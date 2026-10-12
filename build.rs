fn main() {
    const ICON: &str = "assets/brand/bawkshot.ico";
    println!("cargo:rerun-if-changed={ICON}");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    if windows && std::path::Path::new(ICON).exists() {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon(ICON)
            .set("FileDescription", "bawkshot")
            .set("ProductName", "bawkshot");
        resource.compile().expect("compile Windows resources");
    }
}
