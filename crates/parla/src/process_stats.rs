//! The process self-sampler: memory, CPU time and idle wakeups. The perf
//! harness reports it, and the diagnostics export includes it.

/// One reading of the process counters.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    /// macOS `phys_footprint`, Windows private working set, Linux PSS.
    pub memory_mb: f64,
    /// User plus system CPU seconds since start.
    pub cpu_s: f64,
    /// Idle wakeups since start (macOS package idle wakeups; Linux context
    /// switches; not reported on Windows).
    pub wakeups: f64,
}

/// Reads this process's counters, or `None` where the OS reports none.
pub fn sample() -> Option<Sample> {
    imp::sample()
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Sample;
    use std::ffi::c_void;

    /// `rusage_info_v4` from `<libproc.h>`.
    #[repr(C)]
    #[derive(Default)]
    struct RusageInfoV4 {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        pkg_idle_wkups: u64,
        interrupt_wkups: u64,
        pageins: u64,
        wired_size: u64,
        resident_size: u64,
        phys_footprint: u64,
        proc_start_abstime: u64,
        proc_exit_abstime: u64,
        child_user_time: u64,
        child_system_time: u64,
        child_pkg_idle_wkups: u64,
        child_interrupt_wkups: u64,
        child_pageins: u64,
        child_elapsed_abstime: u64,
        diskio_bytesread: u64,
        diskio_byteswritten: u64,
        cpu_time_qos: [u64; 6],
        billed_system_time: u64,
        serviced_system_time: u64,
        logical_writes: u64,
        lifetime_max_phys_footprint: u64,
        instructions: u64,
        cycles: u64,
        billed_energy: u64,
        serviced_energy: u64,
        interval_max_phys_footprint: u64,
        runnable_time: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Timebase {
        number: u32,
        denom: u32,
    }

    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut c_void) -> i32;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }

    const RUSAGE_INFO_V4: i32 = 4;

    pub fn sample() -> Option<Sample> {
        let mut ri = RusageInfoV4::default();
        let mut tb = Timebase::default();
        // SAFETY: both buffers are valid, correctly sized and live for the call.
        let ok = unsafe {
            proc_pid_rusage(
                std::process::id() as i32,
                RUSAGE_INFO_V4,
                (&mut ri as *mut RusageInfoV4).cast(),
            ) == 0
                && mach_timebase_info(&mut tb) == 0
        };
        if !ok || tb.denom == 0 {
            return None;
        }
        let ns = |t: u64| t as f64 * f64::from(tb.number) / f64::from(tb.denom);
        Some(Sample {
            memory_mb: ri.phys_footprint as f64 / 1_048_576.0,
            cpu_s: (ns(ri.user_time) + ns(ri.system_time)) / 1e9,
            wakeups: (ri.pkg_idle_wkups + ri.interrupt_wkups) as f64,
        })
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::Sample;

    pub fn sample() -> Option<Sample> {
        let kb = |text: &str, key: &str| -> Option<f64> {
            text.lines()
                .find(|l| l.starts_with(key))?
                .split_whitespace()
                .nth(1)?
                .parse()
                .ok()
        };
        let smaps = std::fs::read_to_string("/proc/self/smaps_rollup").ok()?;
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        // Fields after the `)` that ends the command name: utime and stime are 14 and 15.
        let after = stat.rsplit_once(')')?.1;
        let f: Vec<&str> = after.split_whitespace().collect();
        let ticks: f64 = f.get(11)?.parse::<f64>().ok()? + f.get(12)?.parse::<f64>().ok()?;
        Some(Sample {
            memory_mb: kb(&smaps, "Pss:")? / 1024.0,
            cpu_s: ticks / 100.0,
            wakeups: kb(&status, "voluntary_ctxt_switches:").unwrap_or(0.0)
                + kb(&status, "nonvoluntary_ctxt_switches:").unwrap_or(0.0),
        })
    }
}

#[cfg(windows)]
mod imp {
    use super::Sample;

    #[repr(C)]
    #[derive(Default)]
    struct MemoryCountersEx {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    impl FileTime {
        /// 100 ns ticks.
        fn ticks(self) -> f64 {
            ((u64::from(self.high) << 32) | u64::from(self.low)) as f64
        }
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(h: isize, c: *mut MemoryCountersEx, cb: u32) -> i32;
        fn GetProcessTimes(
            h: isize,
            create: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }

    pub fn sample() -> Option<Sample> {
        let mut mem = MemoryCountersEx::default();
        mem.cb = std::mem::size_of::<MemoryCountersEx>() as u32;
        let mut c = FileTime::default();
        let (mut e, mut k, mut u) = (c, c, c);
        // SAFETY: the pseudo-handle is always valid and the buffers live for the call.
        let ok = unsafe {
            let h = GetCurrentProcess();
            K32GetProcessMemoryInfo(h, &mut mem, mem.cb) != 0
                && GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0
        };
        ok.then(|| Sample {
            memory_mb: mem.private_usage as f64 / 1_048_576.0,
            cpu_s: (k.ticks() + u.ticks()) / 1e7,
            wakeups: 0.0,
        })
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod imp {
    use super::Sample;

    pub fn sample() -> Option<Sample> {
        None
    }
}
