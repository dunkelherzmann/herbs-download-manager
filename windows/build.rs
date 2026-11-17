// Common Controls 6 (visual styles for the list and buttons) and per-monitor DPI awareness, through the
// manifest the MSVC linker embeds; and the app's icon and file details (a resource script made here, so the
// version matches the app's: version.txt, written by build.ps1).
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' \
             version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
        println!("cargo:rerun-if-changed=../version.txt");
        println!("cargo:rerun-if-changed=icons/app.ico");
        let version = std::fs::read_to_string("../version.txt").ok().map(|v| v.trim().to_string()).unwrap_or_default();
        let mut parts: Vec<u32> = version.split('.').filter_map(|p| p.parse().ok()).collect();
        parts.resize(4, 0);
        let numbers = parts.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let icon = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("icons").join("app.ico");
        let rc = format!(
            r#"#pragma code_page(65001)
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS 0x40004
FILETYPE 1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "dunkelherzmann"
      VALUE "FileDescription", "Herb's Download Manager"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "HerbsDownloadManager"
      VALUE "LegalCopyright", "Copyright (c) Herbert Cisneros. MIT License."
      VALUE "OriginalFilename", "HerbsDownloadManager.exe"
      VALUE "ProductName", "Herb's Download Manager"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
            icon = icon.display().to_string().replace('\\', "\\\\"),
        );
        let path = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("app.rc");
        std::fs::write(&path, rc).unwrap();
        embed_resource::compile(&path, embed_resource::NONE).manifest_optional().unwrap();
    }
}
