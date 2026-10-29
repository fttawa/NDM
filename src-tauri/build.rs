fn main() {
    // Integration tests that initialize Tauri's mock runtime also link TaskDialogIndirect.
    // They need Common Controls v6 just like the application executable does.
    #[cfg(windows)]
    {
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-tests=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
    tauri_build::build()
}
