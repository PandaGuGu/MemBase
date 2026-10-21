fn main() {
    println!("cargo:rerun-if-changed=../../assets/deskkit.ico");
    println!("cargo:rerun-if-changed=build.rs");

    // 只有「在 Windows 上构建 Windows 目标」时才挂资源（图标 + 版本信息）。
    // 在 Linux / WSL 里跑测试时整段跳过，避免 build script 依赖 winres。
    #[cfg(windows)]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            let mut res = winres::WindowsResource::new();
            res.set_icon("../../assets/deskkit.ico");
            res.set("ProductName", "DeskKit");
            res.set("FileDescription", "DeskKit - 内存基准线 + 桌面图标布局");
            res.set("CompanyName", "DeskKit");
            res.set("LegalCopyright", "MIT");
            res.set("OriginalFilename", "deskkit-ui.exe");
            if let Err(e) = res.compile() {
                println!("cargo:warning=资源编译失败（图标会缺省）: {e}");
            }
        }
    }
}
