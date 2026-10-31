use std::path::PathBuf;

use super::Pid;

fn argv0(pid: i32) -> Option<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    let query = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if query != 0 || size == 0 {
        return None;
    }
    let mut buffer = vec![0u8; size];
    let read = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr().cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if read != 0 {
        return None;
    }
    let (argc, rest) = buffer.get(..size)?.split_first_chunk::<4>()?;
    if i32::from_ne_bytes(*argc) == 0 {
        return None;
    }
    let path_end = rest.iter().position(|byte| *byte == 0)?;
    let rest = &rest[path_end..];
    let start = rest.iter().position(|byte| *byte != 0)?;
    let rest = &rest[start..];
    let end = rest.iter().position(|byte| *byte == 0)?;
    let argv0 = std::str::from_utf8(&rest[..end]).ok()?;
    let name = argv0.rsplit('/').next()?.trim_start_matches('-');
    (!name.is_empty()).then(|| name.to_string())
}

#[must_use]
pub fn name(pid: Pid) -> Option<String> {
    let pid = pid.0;
    if let Some(name) = argv0(pid) {
        return Some(name);
    }
    let mut path = [0u8; 4096];
    let capacity = u32::try_from(path.len()).ok()?;
    let written = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), capacity) };
    let written = usize::try_from(written).ok().filter(|&length| length > 0)?;
    let path = std::str::from_utf8(&path[..written]).ok()?;
    Some(path.rsplit('/').next()?.to_string())
}

#[must_use]
pub fn cwd(pid: Pid) -> Option<PathBuf> {
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_vnodepathinfo>()).ok()?;
    let written = unsafe {
        libc::proc_pidinfo(
            pid.0,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if written < size {
        return None;
    }

    let raw = &info.pvi_cdir.vip_path;
    let bytes = unsafe { std::slice::from_raw_parts(raw.as_ptr().cast::<u8>(), size_of_val(raw)) };
    let end = bytes.iter().position(|byte| *byte == 0)?;
    let path = std::str::from_utf8(&bytes[..end]).ok()?;
    Some(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn name_is_the_one_it_was_started_by() {
        let dir = std::env::temp_dir().join(format!("spawnshell-argv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("claude");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink("/bin/sleep", &link).unwrap();

        let mut child = Command::new(&link).arg("5").spawn().unwrap();
        let name = name(Pid(child.id().cast_signed()));
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(name.as_deref(), Some("claude"));
    }
}
