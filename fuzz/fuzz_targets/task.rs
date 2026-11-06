#![no_main]
libfuzzer_sys::fuzz_target!(|text: &str| {
    let _ = insi_core::Config::parse(text);
    if let Ok(task) = toml::from_str::<insi_core::Task>(text) {
        let _ = task.validate();
        let _ = task.check_argv();
    }
});
