//! Crash-isolated execution of the Hope:RE engines.
//!
//! ONNX Runtime runs inside a child process (`shieldcraft-cli hope-worker`) so a native crash —
//! Hope:RE reports access violations with DirectML on these models — ends that process with
//! an error message instead of taking the whole application down.
//!
//! Protocol: the child prints `PROGRESS <0..1> <message>` lines, then `RESULT <json>`, or
//! `ERROR <message>` with a non-zero exit code. Cancelling kills the child.

use super::hope::HopeParams;
use crate::engine::{CancelToken, EngineError, ProgressFn};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Serialize, Deserialize)]
pub struct HopeRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub params: HopeParams,
    /// Optional selection mask (grayscale PNG, 255 = protect). Absent = whole image.
    #[serde(default)]
    pub mask: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct HopeResponse {
    pub execution_provider: String,
    pub tiles: u32,
    pub evaluations: u64,
    pub seconds: f64,
    /// Mean surrogate-model loss per tile before and after optimisation (lower after = the
    /// optimiser moved the model in the intended direction).
    pub loss_before: f32,
    pub loss_after: f32,
    #[serde(default)]
    pub tiles_extended: u32,
    #[serde(default)]
    pub tiles_unconverged: u32,
    #[serde(default)]
    pub total_iterations: u64,
    #[serde(default)]
    pub tiles_skipped: u32,
}

/// The official ONNX Runtime DLL shipped next to the executables.
pub fn runtime_dll() -> Option<PathBuf> {
    Some(crate::paths::exe_dir()?.join("onnxruntime.dll"))
}

pub fn worker_exe() -> Option<PathBuf> {
    let dir = crate::paths::exe_dir()?;
    let p = dir.join("shieldcraft-cli.exe");
    p.is_file().then_some(p)
}

pub fn run_hope(
    img: &RgbaImage,
    params: &HopeParams,
    mask: Option<&crate::selection::Mask>,
    progress: ProgressFn,
    cancel: &CancelToken,
) -> Result<(RgbaImage, HopeResponse), EngineError> {
    let exe = worker_exe().ok_or_else(|| EngineError::Unavailable("shieldcraft-cli.exe not found next to the application".into()))?;
    let tmp = crate::paths::temp_dir();
    let tag = format!("{}-{}", std::process::id(), crate::util::fresh_seed());
    let input = tmp.join(format!("hope-in-{tag}.png"));
    let output = tmp.join(format!("hope-out-{tag}.png"));
    let mask_path = tmp.join(format!("hope-mask-{tag}.png"));
    let req_path = tmp.join(format!("hope-req-{tag}.json"));
    let cleanup = || {
        for p in [&input, &output, &mask_path, &req_path] {
            let _ = std::fs::remove_file(p);
        }
    };
    img.save(&input).map_err(|e| EngineError::Failed(format!("Cannot write temporary input: {e}")))?;
    let mask_arg = match mask {
        Some(m) => {
            m.save(&mask_path).map_err(|e| EngineError::Failed(format!("Cannot write temporary mask: {e}")))?;
            Some(mask_path.clone())
        }
        None => None,
    };
    let req = HopeRequest { input: input.clone(), output: output.clone(), params: params.clone(), mask: mask_arg };
    std::fs::write(&req_path, serde_json::to_vec(&req).unwrap()).map_err(|e| EngineError::Failed(e.to_string()))?;

    let mut cmd = Command::new(&exe);
    cmd.arg("hope-worker").arg("--request").arg(&req_path).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| {
        cleanup();
        EngineError::Failed(format!("Could not start the worker process: {e}"))
    })?;

    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = crossbeam_channel::unbounded::<String>();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let stderr = child.stderr.take().unwrap();
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut s);
        s
    });

    let mut result: Option<HopeResponse> = None;
    let mut error: Option<String> = None;
    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            cleanup();
            return Err(EngineError::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                if let Some(rest) = line.strip_prefix("PROGRESS ") {
                    let (f, msg) = rest.split_once(' ').unwrap_or((rest, ""));
                    progress(f.parse().unwrap_or(0.0), msg);
                } else if let Some(rest) = line.strip_prefix("RESULT ") {
                    result = serde_json::from_str(rest).ok();
                } else if let Some(rest) = line.strip_prefix("ERROR ") {
                    error = Some(rest.to_string());
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().map_err(|e| EngineError::Failed(e.to_string()))?;
    let _ = reader.join();
    let stderr_text = err_reader.join().unwrap_or_default();

    let outcome = if let Some(e) = error {
        Err(crate::engine::translate_error(&e))
    } else if !status.success() || result.is_none() {
        let code = status.code().map(|c| format!("{c:#x}")).unwrap_or_else(|| "unknown".into());
        let tail: String = stderr_text.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        Err(EngineError::Failed(format!(
            "The worker process stopped unexpectedly (exit code {code}). If DirectML was selected, switch to CPU.\n{tail}"
        )))
    } else {
        image::open(&output)
            .map(|i| (i.to_rgba8(), result.unwrap()))
            .map_err(|e| EngineError::Failed(format!("Worker output could not be read: {e}")))
    };
    cleanup();
    outcome
}
