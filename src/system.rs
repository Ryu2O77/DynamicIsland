//! Estadísticas del sistema con APIs nativas de Windows.

use std::ffi::c_void;
use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::{FILETIME, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

#[derive(Clone, Copy, Default)]
pub struct Sys {
    pub cpu: f32,
    pub ram_pct: f32,
    pub ram_used: u64,
    pub ram_total: u64,
    pub disk_pct: f32,
    pub disk_free: u64,
    pub disk_total: u64,
    pub net_down: f64,
    pub net_up: f64,
    pub uptime: u64,
}

pub struct Sampler {
    prev_cpu: Option<(u64, u64)>,
    prev_net: Option<(u64, u64, Instant)>,
}

impl Default for Sampler {
    fn default() -> Self {
        Self { prev_cpu: None, prev_net: None }
    }
}

fn ft(v: FILETIME) -> u64 {
    ((v.dwHighDateTime as u64) << 32) | v.dwLowDateTime as u64
}

impl Sampler {
    pub fn sample(&mut self) -> Sys {
        let mut s = Sys::default();

        unsafe {
            let mut idle = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            if GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).is_ok() {
                let i = ft(idle);
                let total = ft(kernel) + ft(user);
                if let Some((pi, pt)) = self.prev_cpu {
                    let di = i.saturating_sub(pi);
                    let dt = total.saturating_sub(pt);
                    if dt > 0 {
                        s.cpu = (100.0 * (1.0 - di as f64 / dt as f64)).clamp(0.0, 100.0) as f32;
                    }
                }
                self.prev_cpu = Some((i, total));
            }

            let mut m = MEMORYSTATUSEX {
                dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
                ..Default::default()
            };
            if GlobalMemoryStatusEx(&mut m).is_ok() {
                s.ram_pct = m.dwMemoryLoad as f32;
                s.ram_total = m.ullTotalPhys;
                s.ram_used = m.ullTotalPhys.saturating_sub(m.ullAvailPhys);
            }

            let mut free = 0u64;
            let mut total = 0u64;
            if GetDiskFreeSpaceExW(w!("C:\\"), None, Some(&mut total), Some(&mut free)).is_ok() && total > 0 {
                s.disk_total = total;
                s.disk_free = free;
                s.disk_pct = (100.0 * (1.0 - free as f64 / total as f64)) as f32;
            }

            let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
            if GetIfTable2(&mut table) == NO_ERROR && !table.is_null() {
                let n = (*table).NumEntries as usize;
                let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), n);
                let (mut rx, mut tx) = (0u64, 0u64);
                for r in rows {
                    // 24 = IF_TYPE_SOFTWARE_LOOPBACK
                    if r.Type == 24 || r.OperStatus != IfOperStatusUp {
                        continue;
                    }
                    rx = rx.saturating_add(r.InOctets);
                    tx = tx.saturating_add(r.OutOctets);
                }
                FreeMibTable(table as *const c_void);
                let now = Instant::now();
                if let Some((prx, ptx, pt)) = self.prev_net {
                    let dt = now.duration_since(pt).as_secs_f64().max(0.001);
                    s.net_down = rx.saturating_sub(prx) as f64 / dt;
                    s.net_up = tx.saturating_sub(ptx) as f64 / dt;
                }
                self.prev_net = Some((rx, tx, now));
            }

            s.uptime = GetTickCount64() / 1000;
        }

        s
    }
}
