mod cli;

fn main() {
    if let Err(error) = close_inherited_descriptors() {
        eprintln!("parcela: cannot release inherited descriptors: {error}");
        std::process::exit(1);
    }
    cli::main();
}

fn close_inherited_descriptors() -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let directory = "/dev/fd";
    #[cfg(target_os = "linux")]
    let directory = "/proc/self/fd";
    let mut descriptors = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        if let Some(fd) = entry?
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
            && fd > 2
        {
            descriptors.push(fd);
        }
    }
    // Before starting threads, release only this process's inherited copies. A caller's transient
    // listener must not survive inside Parcela or Caddy. The CLI's descriptor contract is stdio only.
    for descriptor in descriptors {
        // SAFETY: no threads or new descriptor allocations race this startup cleanup.
        unsafe {
            libc::close(descriptor);
        }
    }
    Ok(())
}
