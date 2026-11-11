fn main() {
    println!("cargo:rerun-if-changed=assets/controller.ico");
    println!("cargo:rerun-if-changed=assets/dark");
    println!("cargo:rerun-if-changed=assets/models");
    println!("cargo:rerun-if-changed=assets/buttons");
    println!("cargo:rerun-if-changed=ui/appwindow.slint");

    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/controller.ico");
        res.set("FileDescription", "IBBE Controller Manager");
        res.set("ProductName", "IBBE Controller Manager");
        res.set("CompanyName", "Gallery Inc.");
        res.set("FileVersion", env!("CARGO_PKG_VERSION"));
        res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
        res.set_manifest(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
          <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
            <security>
              <requestedPrivileges>
                <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
              </requestedPrivileges>
            </security>
          </trustInfo>
          <application xmlns="urn:schemas-microsoft-com:asm.v3">
            <windowsSettings>
              <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
              <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
            </windowsSettings>
          </application>
        </assembly>"#);
        res.compile().expect("failed to compile Windows resources");
    }

    slint_build::compile("ui/appwindow.slint").expect("Slint compilation failed");
}
