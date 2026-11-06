//! Own the process tree before resuming its initially suspended thread.
use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
type Handle = *mut c_void;

#[repr(C)]
#[derive(Default)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    min_working_set: usize,
    max_working_set: usize,
    active_processes: u32,
    affinity: usize,
    priority: u32,
    scheduling: u32,
}
#[repr(C)]
#[derive(Default)]
struct Limits {
    basic: BasicLimits,
    io: [u64; 6],
    memory: [usize; 4],
}
#[repr(C)]
struct ThreadEntry {
    size: u32,
    usage: u32,
    id: u32,
    owner: u32,
    priority: i32,
    delta: i32,
    flags: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(job: Handle, class: i32, info: *const c_void, size: u32) -> i32;
    fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
    fn TerminateJobObject(job: Handle, code: u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
    fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
    fn OpenThread(access: u32, inherit: i32, id: u32) -> Handle;
    fn ResumeThread(thread: Handle) -> u32;
}

pub(super) struct Job(Handle);
impl Job {
    pub fn new() -> Result<Self, String> {
        // SAFETY: null attributes/name create an unnamed, non-inherited job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let job = Self(handle);
        let mut limits = Limits::default();
        limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                                     // SAFETY: repr(C) matches JOBOBJECT_EXTENDED_LIMIT_INFORMATION.
        if unsafe {
            SetInformationJobObject(
                handle,
                9,
                (&limits as *const Limits).cast(),
                std::mem::size_of::<Limits>() as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(job)
    }
    pub fn start(&self, child: &std::process::Child) -> Result<(), String> {
        // SAFETY: child is alive and suspended, and both handles are owned.
        if unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle()) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // std does not expose the initial thread handle. Enumerate it while
        // suspended: no worker code can create another thread or descendant.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(4, 0); // TH32CS_SNAPTHREAD
            if snapshot as isize == -1 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut entry = ThreadEntry {
                size: std::mem::size_of::<ThreadEntry>() as u32,
                usage: 0,
                id: 0,
                owner: 0,
                priority: 0,
                delta: 0,
                flags: 0,
            };
            let mut found = Thread32First(snapshot, &mut entry);
            let mut resumed = false;
            while found != 0 {
                if entry.owner == child.id() {
                    let thread = OpenThread(2, 0, entry.id); // THREAD_SUSPEND_RESUME
                    if !thread.is_null() {
                        resumed = ResumeThread(thread) != u32::MAX;
                        CloseHandle(thread);
                    }
                    break;
                }
                found = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            if !resumed {
                return Err("could not resume worker's initial thread".into());
            }
        }
        Ok(())
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: this object exclusively owns the job handle.
        unsafe {
            TerminateJobObject(self.0, 1);
            CloseHandle(self.0);
        }
    }
}
