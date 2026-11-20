//! Hardware detection and the default model choice (DESIGN 5.1 Detect).

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::app::settings::Backend;
use crate::inference::process::CREATE_NO_WINDOW;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuInfo {
    pub name: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub avx2: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuDevice {
    /// llama.cpp device id, e.g. `Vulkan0`.
    pub id: String,
    pub name: String,
    pub total_mb: u64,
    pub free_mb: u64,
    /// Dedicated VRAM reported by DXGI for the matching adapter, if found.
    pub dedicated_mb: Option<u64>,
    /// Integrated GPU (DXGI dedicated memory < 1 GiB): uses shared system RAM.
    pub integrated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareInfo {
    pub cpu: CpuInfo,
    pub ram_total_mb: u64,
    pub ram_available_mb: u64,
    pub gpus: Vec<GpuDevice>,
    /// Error text when `llama-server --list-devices` could not run.
    pub list_devices_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub model_id: String,
    pub backend: Backend,
    pub device: Option<String>,
    pub threads: u32,
    pub reason: String,
    pub warnings: Vec<String>,
}

pub const MODEL_2B: &str = "index-translate-2b-q4";
pub const MODEL_7B: &str = "hy-mt2-7b-q4";
pub const MODEL_9B: &str = "index-translate-9b-q4";

/// Parse `llama-server --list-devices`:
/// `  Vulkan0: NVIDIA GeForce RTX 4060 Laptop GPU (7957 MiB, 7189 MiB free)`
pub fn parse_list_devices(out: &str) -> Vec<GpuDevice> {
    out.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (id, rest) = line.split_once(": ")?;
            if !id.starts_with("Vulkan") {
                return None;
            }
            let open = rest.rfind('(')?;
            let name = rest[..open].trim().to_string();
            let inner = rest[open + 1..].trim_end_matches(')');
            let mut nums = inner
                .split(',')
                .filter_map(|p| p.trim().split_whitespace().next()?.parse::<u64>().ok());
            let total_mb = nums.next()?;
            let free_mb = nums.next().unwrap_or(total_mb);
            Some(GpuDevice {
                id: id.to_string(),
                name,
                total_mb,
                free_mb,
                dedicated_mb: None,
                integrated: false,
            })
        })
        .collect()
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// (description, dedicated VRAM MB) for each hardware DXGI adapter.
fn dxgi_adapters() -> Vec<(String, u64)> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1};
    let mut out = Vec::new();
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let Ok(desc) = adapter.GetDesc1() else { continue };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
            let name = String::from_utf16_lossy(&desc.Description[..len]);
            out.push((name, (desc.DedicatedVideoMemory / (1024 * 1024)) as u64));
        }
    }
    out
}

fn annotate_with_dxgi(gpus: &mut [GpuDevice], adapters: &[(String, u64)]) {
    for g in gpus.iter_mut() {
        let gn = norm(&g.name);
        if let Some((_, mb)) = adapters.iter().find(|(n, _)| {
            let an = norm(n);
            !an.is_empty() && (an.contains(&gn) || gn.contains(&an))
        }) {
            g.dedicated_mb = Some(*mb);
            g.integrated = *mb < 1024;
        }
    }
}

async fn list_devices(llama_server: &Path) -> Result<Vec<GpuDevice>, String> {
    let mut cmd = tokio::process::Command::new(llama_server);
    cmd.arg("--list-devices").creation_flags(CREATE_NO_WINDOW).kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(30), cmd.output())
        .await
        .map_err(|_| "llama-server --list-devices 超时".to_string())?
        .map_err(|e| format!("无法运行 llama-server：{e}"))?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(parse_list_devices(&text))
}

pub async fn detect(llama_server: &Path) -> HardwareInfo {
    let (gpus, list_devices_error) = match list_devices(llama_server).await {
        Ok(g) => (g, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    let mut gpus = gpus;
    annotate_with_dxgi(&mut gpus, &dxgi_adapters());

    let (name, logical, ram_total, ram_avail) = match sysinfo::System::new() {
        Ok(mut sys) => {
            sys.refresh_memory();
            sys.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
            (
                sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
                sys.cpus().len().max(1) as u32,
                sys.total_memory(),
                sys.available_memory(),
            )
        }
        Err(e) => {
            tracing::warn!("sysinfo unavailable: {e:?}");
            (String::new(), std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1), 0, 0)
        }
    };
    let physical = sysinfo::System::physical_core_count().map(|n| n as u32).unwrap_or(logical);

    HardwareInfo {
        cpu: CpuInfo {
            name,
            physical_cores: physical,
            logical_cores: logical,
            avx2: std::arch::is_x86_feature_detected!("avx2"),
        },
        ram_total_mb: ram_total / (1024 * 1024),
        ram_available_mb: ram_avail / (1024 * 1024),
        gpus,
        list_devices_error,
    }
}

/// Default choice rules (DESIGN 5.1-2). Integrated GPUs share system RAM and are
/// not counted as VRAM, so they take the CPU path.
pub fn recommend(hw: &HardwareInfo) -> Recommendation {
    let mut warnings = Vec::new();
    let threads = hw.cpu.physical_cores.max(1);
    if !hw.cpu.avx2 {
        warnings.push("CPU 不支持 AVX2，CPU 推理会较慢".into());
    }
    let best = hw
        .gpus
        .iter()
        .filter(|g| !g.integrated)
        .max_by_key(|g| g.free_mb);
    let vram_gb = best.map(|g| g.free_mb as f64 / 1024.0).unwrap_or(0.0);
    let ram_gb = hw.ram_total_mb as f64 / 1024.0;

    // Measured with a 4K context (DESIGN 10.6): 9B needs 5.1 GB, 2B 1.5 GB.
    // Hy-MT2-7B needs as much VRAM as 9B but 9B is the stronger model, so 7B
    // is never the default (still selectable in Customize).
    let (mut model, backend) = if vram_gb >= 5.5 {
        (MODEL_9B, Backend::Vulkan)
    } else if vram_gb >= 2.0 {
        (MODEL_2B, Backend::Vulkan)
    } else {
        (MODEL_2B, Backend::Cpu)
    };
    if ram_gb < 8.0 && model != MODEL_2B {
        warnings.push(format!("内存仅 {ram_gb:.1} GB，建议使用轻量模型"));
        model = MODEL_2B;
    }

    let reason = match (backend, best) {
        (Backend::Vulkan, Some(g)) => {
            format!("显卡 {} 可用显存 {:.1} GB，使用 GPU 加速", g.name, vram_gb)
        }
        _ if hw.gpus.iter().any(|g| g.integrated) && best.is_none() => {
            format!("仅检测到核显，使用 CPU（{threads} 线程）")
        }
        _ if best.is_some() => format!("显存 {vram_gb:.1} GB 不足 2 GB，使用 CPU（{threads} 线程）"),
        _ => format!("未检测到可用的独立显卡，使用 CPU（{threads} 线程）"),
    };

    Recommendation {
        model_id: model.to_string(),
        backend,
        device: if backend == Backend::Vulkan { best.map(|g| g.id.clone()) } else { None },
        threads,
        reason,
        warnings,
    }
}

/// Free bytes on the volume holding `path` (or its nearest existing ancestor).
pub fn disk_free(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::core::PCWSTR;
    let mut p = path;
    while !p.exists() {
        p = p.parent()?;
    }
    let mut wide: Vec<u16> = p.as_os_str().encode_wide().collect();
    if wide.last() != Some(&(b'\\' as u16)) {
        wide.push(b'\\' as u16);
    }
    wide.push(0);
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None).ok()? };
    Some(free)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(gpus: Vec<GpuDevice>, ram_gb: u64) -> HardwareInfo {
        HardwareInfo {
            cpu: CpuInfo { name: "x".into(), physical_cores: 8, logical_cores: 16, avx2: true },
            ram_total_mb: ram_gb * 1024,
            ram_available_mb: ram_gb * 512,
            gpus,
            list_devices_error: None,
        }
    }

    fn gpu(id: &str, free_mb: u64, integrated: bool) -> GpuDevice {
        GpuDevice {
            id: id.into(),
            name: id.into(),
            total_mb: free_mb,
            free_mb,
            dedicated_mb: None,
            integrated,
        }
    }

    #[test]
    fn parses_list_devices() {
        let out = "ggml_vulkan: Found 2 Vulkan devices:\nAvailable devices:\n  Vulkan0: NVIDIA GeForce RTX 4060 Laptop GPU (7957 MiB, 7189 MiB free)\n  Vulkan1: AMD Radeon(TM) 610M (2048 MiB, 1900 MiB free)\n";
        let d = parse_list_devices(out);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].id, "Vulkan0");
        assert_eq!(d[0].name, "NVIDIA GeForce RTX 4060 Laptop GPU");
        assert_eq!((d[0].total_mb, d[0].free_mb), (7957, 7189));
        assert_eq!(d[1].name, "AMD Radeon(TM) 610M");
        assert!(parse_list_devices("Available devices:\n").is_empty());
    }

    #[test]
    fn dxgi_matching_flags_integrated() {
        let mut g = vec![gpu("Vulkan0", 7000, false), gpu("Vulkan1", 8000, false)];
        g[0].name = "NVIDIA GeForce RTX 4060 Laptop GPU".into();
        g[1].name = "Intel(R) Arc(TM) Graphics".into();
        annotate_with_dxgi(
            &mut g,
            &[("NVIDIA GeForce RTX 4060 Laptop GPU".into(), 7900), ("Intel(R) Arc(TM) Graphics".into(), 128)],
        );
        assert!(!g[0].integrated);
        assert!(g[1].integrated);
    }

    #[test]
    fn recommendation_tiers() {
        let r = recommend(&hw(vec![gpu("Vulkan0", 7189, false)], 16));
        assert_eq!((r.model_id.as_str(), r.backend), (MODEL_9B, Backend::Vulkan));
        assert_eq!(r.device.as_deref(), Some("Vulkan0"));
        // A 6 GB card (~5.6 GB free) runs 9B; 7B is never picked automatically.
        assert_eq!(recommend(&hw(vec![gpu("Vulkan0", 5700, false)], 16)).model_id, MODEL_9B);
        assert_eq!(recommend(&hw(vec![gpu("Vulkan0", 5500, false)], 16)).model_id, MODEL_2B);
        for free in [1000, 2500, 5000, 5700, 7000, 12000] {
            assert_ne!(recommend(&hw(vec![gpu("Vulkan0", free, false)], 16)).model_id, MODEL_7B);
        }
        let r = recommend(&hw(vec![gpu("Vulkan0", 3000, false)], 16));
        assert_eq!((r.model_id.as_str(), r.backend), (MODEL_2B, Backend::Vulkan));
        let r = recommend(&hw(vec![gpu("Vulkan0", 1500, false)], 16));
        assert_eq!((r.model_id.as_str(), r.backend, r.threads), (MODEL_2B, Backend::Cpu, 8));
        assert_eq!(recommend(&hw(vec![], 16)).backend, Backend::Cpu);
        // Integrated GPU with big shared memory is ignored; discrete picked over it.
        let r = recommend(&hw(vec![gpu("Vulkan0", 9000, true), gpu("Vulkan1", 3000, false)], 16));
        assert_eq!((r.model_id.as_str(), r.device.as_deref()), (MODEL_2B, Some("Vulkan1")));
        assert_eq!(recommend(&hw(vec![gpu("Vulkan0", 9000, true)], 16)).backend, Backend::Cpu);
        // Low RAM caps to 2B.
        assert_eq!(recommend(&hw(vec![gpu("Vulkan0", 7189, false)], 6)).model_id, MODEL_2B);
    }
}

