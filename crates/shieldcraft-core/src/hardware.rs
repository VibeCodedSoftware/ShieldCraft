//! Hardware summary for the status bar and diagnostics.

#[derive(Clone, Debug, Default)]
pub struct Hardware {
    pub cpu: String,
    pub cores: usize,
    pub ram_gb: f32,
    pub gpu: Option<String>,
    pub os: String,
}

pub fn detect() -> Hardware {
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();
    let cpu = sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_else(|| "Unknown CPU".into());
    Hardware {
        cpu,
        cores: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        ram_gb: sys.total_memory() as f32 / 1_073_741_824.0,
        gpu: None,
        os: format!("{} {}", sysinfo::System::name().unwrap_or_default(), sysinfo::System::os_version().unwrap_or_default()),
    }
}
