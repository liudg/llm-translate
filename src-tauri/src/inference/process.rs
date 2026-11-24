//! Child-process helpers: no console window, and a kill-on-close Job Object so
//! llama-server never outlives the app (DESIGN 5.2).

use std::sync::OnceLock;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Job(HANDLE);
// The handle is only used for AssignProcessToJobObject, which is thread-safe.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

fn job() -> Option<&'static Job> {
    static JOB: OnceLock<Option<Job>> = OnceLock::new();
    JOB.get_or_init(|| unsafe {
        let h = CreateJobObjectW(None, None).ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .ok()?;
        // Intentionally never closed: the OS closes it when our process exits,
        // which kills every assigned child.
        Some(Job(h))
    })
    .as_ref()
}

/// Put a spawned child into the app-wide job.
pub fn attach_to_job(child: &tokio::process::Child) {
    let Some(raw) = child.raw_handle() else { return };
    if let Some(job) = job() {
        if let Err(e) = unsafe { AssignProcessToJobObject(job.0, HANDLE(raw)) } {
            tracing::warn!("AssignProcessToJobObject failed: {e}");
        }
    }
}
