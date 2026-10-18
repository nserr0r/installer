use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command as Process;
use std::process::Stdio;

use crate::error::Error;
use crate::plan;
use crate::plan::Command;
use crate::plan::File;
use crate::plan::Input;
use crate::plan::Step;

pub trait Runner {
    fn run(&mut self, command: &Command) -> Result<(), Error>;
}

pub struct System;

impl Runner for System {
    fn run(&mut self, command: &Command) -> Result<(), Error> {
        let spawn_error = |source| Error::Spawn {
            program: command.program.clone(),
            source,
        };
        let stdin = if command.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        };

        let mut child = Process::new(&command.program)
            .args(&command.args)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(spawn_error)?;

        if let (Some(input), Some(mut pipe)) = (&command.input, child.stdin.take()) {
            let bytes = match input {
                Input::Script(text) => text.as_bytes(),
                Input::Secret(secret) => secret.expose().as_bytes(),
            };

            pipe.write_all(bytes).map_err(spawn_error)?;
        }

        let output = child.wait_with_output().map_err(spawn_error)?;

        if output.status.success() {
            return Ok(());
        }

        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));

        Err(Error::Failed {
            command: command.to_string(),
            status: output.status,
            output: text.trim().to_string(),
        })
    }
}

/// Каталоги, где лежат утилиты, даже если `PATH` обычного пользователя их не включает.
const TOOL_DIRS: [&str; 4] = ["/usr/sbin", "/sbin", "/usr/bin", "/bin"];

/// Есть ли у установщика исполняемый файл с таким именем.
pub fn installed(program: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();

    std::env::split_paths(&path)
        .chain(TOOL_DIRS.iter().map(PathBuf::from))
        .any(|dir| {
            fs::metadata(dir.join(program))
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

pub fn execute(
    steps: &[Step],
    root: &Path,
    runner: &mut dyn Runner,
    mut progress: impl FnMut(usize, &Step),
) -> Result<(), Error> {
    for (index, step) in steps.iter().enumerate() {
        progress(index, step);

        match step {
            Step::Run(command) => runner.run(command)?,
            Step::Write(file) => write_file(root, file)?,
            Step::Release(disks) => runner.run(&plan::release_command(disks))?,
        }
    }

    Ok(())
}

fn write_file(root: &Path, file: &File) -> Result<(), Error> {
    let path = root.join(file.path.trim_start_matches('/'));
    let write_error = |source| Error::Write {
        path: path.clone(),
        source,
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(write_error)?;
    }

    fs::write(&path, &file.contents).map_err(write_error)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(file.mode)).map_err(write_error)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process;

    use super::*;
    use crate::config::Secret;

    struct Recorder {
        seen: Vec<String>,
        fail_on: Option<String>,
    }

    impl Runner for Recorder {
        fn run(&mut self, command: &Command) -> Result<(), Error> {
            self.seen.push(command.program.clone());

            if self.fail_on.as_deref() == Some(command.program.as_str()) {
                return Err(Error::Invalid("сбой".into()));
            }

            Ok(())
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("moonshine-installer-{name}-{}", process::id()));

        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();

        path
    }

    fn run(program: &str) -> Step {
        Step::Run(Command::new(program, Vec::<String>::new()))
    }

    #[test]
    fn steps_run_in_order_and_stop_at_the_first_failure() {
        let mut recorder = Recorder {
            seen: Vec::new(),
            fail_on: Some("b".into()),
        };
        let mut reported = Vec::new();
        let result = execute(
            &[run("a"), run("b"), run("c")],
            Path::new("/nonexistent"),
            &mut recorder,
            |index, _| reported.push(index),
        );

        assert!(result.is_err());
        assert_eq!(recorder.seen, ["a", "b"]);
        assert_eq!(reported, [0, 1]);
    }

    #[test]
    fn files_are_written_under_the_root_with_their_mode() {
        let root = temp_dir("files");
        let step = Step::Write(File {
            path: "/root/.ssh/authorized_keys".into(),
            contents: "key\n".into(),
            mode: 0o600,
        });
        let mut recorder = Recorder {
            seen: Vec::new(),
            fail_on: None,
        };

        execute(&[step], &root, &mut recorder, |_, _| {}).unwrap();

        let path = root.join("root/.ssh/authorized_keys");

        assert_eq!(fs::read_to_string(&path).unwrap(), "key\n");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(recorder.seen.is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_system_runner_passes_stdin_and_reports_failures() {
        let mut system = System;
        let cat = Command {
            program: "cat".into(),
            args: Vec::new(),
            input: Some(Input::Secret(Secret::new("data"))),
        };

        assert!(system.run(&cat).is_ok());

        let failing = Command::new("sh", ["-c", "echo boom >&2; exit 3"]);
        let error = system.run(&failing).unwrap_err().to_string();

        assert!(error.contains("boom") && error.contains("sh -c"), "{error}");
        assert!(matches!(
            system.run(&Command::new(
                "moonshine-no-such-program",
                Vec::<String>::new()
            )),
            Err(Error::Spawn { .. })
        ));
    }

    #[test]
    fn release_stops_the_deepest_devices_first_on_fake_tools() {
        let bin = temp_dir("release");
        let log = bin.join("log");
        let tools = [
            (
                "lsblk",
                "printf '%s\\n' '/dev/sda disk ' '/dev/sda1 part [SWAP]' '/dev/sda2 part ' '/dev/md0 raid1 ' '/dev/mapper/vg-root lvm /mnt/x' '/dev/mapper/crypt crypt '",
            ),
            ("swapoff", "echo swapoff $* >> \"$LOG\""),
            ("mountpoint", "exit 0"),
            ("umount", "echo umount $* >> \"$LOG\""),
            ("dmsetup", "echo dmsetup $* >> \"$LOG\""),
            ("mdadm", "echo mdadm $* >> \"$LOG\""),
        ];

        for (name, body) in tools {
            let path = bin.join(name);

            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let command = plan::release_command(&["/dev/sda".to_string()]);
        let status = Process::new(&command.program)
            .args(&command.args)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("LOG", &log)
            .status()
            .unwrap();

        assert!(status.success());
        assert_eq!(
            fs::read_to_string(&log)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            [
                "dmsetup remove /dev/mapper/crypt",
                "umount -R /mnt/x",
                "dmsetup remove /dev/mapper/vg-root",
                "mdadm --stop /dev/md0",
                "swapoff /dev/sda1",
            ]
        );
    }

    #[test]
    fn a_failing_release_stops_the_installation() {
        let bin = temp_dir("release-fail");
        let lsblk = bin.join("lsblk");
        let umount = bin.join("umount");
        let mountpoint = bin.join("mountpoint");

        fs::write(&lsblk, "#!/bin/sh\necho '/dev/sda1 part /mnt/x'\n").unwrap();
        fs::write(&umount, "#!/bin/sh\nexit 1\n").unwrap();
        fs::write(&mountpoint, "#!/bin/sh\nexit 0\n").unwrap();

        for path in [&lsblk, &umount, &mountpoint] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let command = plan::release_command(&["/dev/sda".to_string()]);
        let status = Process::new(&command.program)
            .args(&command.args)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .status()
            .unwrap();

        assert!(!status.success());
    }

    #[test]
    fn release_goes_through_the_runner() {
        let mut runner = Recorder {
            seen: Vec::new(),
            fail_on: None,
        };

        execute(
            &[Step::Release(vec!["/dev/sda".into()])],
            Path::new("/nonexistent"),
            &mut runner,
            |_, _| {},
        )
        .unwrap();

        assert_eq!(runner.seen, ["sh"]);
    }

    #[test]
    fn installed_finds_real_programs_only() {
        assert!(installed("sh"));
        assert!(!installed("no-such-tool-moonshine"));
    }
}
