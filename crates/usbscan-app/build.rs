fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("cupertino".into());
    slint_build::compile_with_config("ui/main.slint", config).expect("compile Slint UI");

    #[cfg(windows)]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("../../assets/icons/usbscan.ico");
        resource.set("ProductName", "UsbScan");
        resource.set("FileDescription", "UsbScan document scanner");
        resource.set("OriginalFilename", "usbscan.exe");
        resource.compile().expect("compile Windows resources");
    }
}
