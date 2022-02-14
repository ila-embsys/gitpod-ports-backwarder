#![crate_type = "lib"]

use std::process::Stdio;
use regex::Regex;

use tokio::process::Command;
use tokio::io::{AsyncBufReadExt, BufReader};

use command_group::{AsyncCommandGroup, AsyncGroupChild};

pub enum HostName {
    Default,
    Is(String),
}

pub struct CompanionHandle {
    pub companion_process: AsyncGroupChild,
    pub ssh_config_path: String,
}

pub async fn run_companion(gitpod_host: HostName) -> Option<CompanionHandle> {
    println!("Hello, world!");

    let arg_no_tunnel = "--auto-tunnel=false";
    let arg_gitpod_host = "--gitpod-host";

    let args = {
        match gitpod_host {
            HostName::Is(host) => {
                let ret = vec![arg_no_tunnel.to_string(), arg_gitpod_host.to_string(), host];
                ret
            }
            HostName::Default => {
                let ret = vec![arg_no_tunnel.to_string()];
                ret
            }
        }
    };

    let companion_process = Command::new("gitpod-local-companion-windows.exe")
        .args(args)
        .stderr(Stdio::piped())
        .group_spawn();

    let mut companion_process = match companion_process {
        Ok(companion_process) => companion_process,
        Err(e) => {
            println!("Can not create companion process: {}", e);
            return None;
        }
    };

    let stderr = companion_process.inner().stderr.take();

    let stderr = match stderr {
        Some(stderr) => stderr,
        None => {
            println!("Can not get output pipe from companion");
            return None;
        }
    };

    let ssh_config_path = {
        // let stderr = companion_process.stderr.as_mut()?;
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();

        fn parse_line_for_config(line: String) -> Option<String> {
            println!("Searhing in companion output for ssh_config...");
            println!("[Companion]: {:?}", line);
            let re = Regex::new(r#"ssh_config="(?P<ssh_config>.*)""#).ok();
            let capture = re?.captures(&line);
            let ssh_config = capture?.name("ssh_config")?.as_str();
            println!("Matched ssh_config: {}", ssh_config);
            let ssh_config = ssh_config.to_string();
            Some(ssh_config)
        }

        fn parse_line_for_listening(line: String) -> bool {
            println!("Searhing in companion output for listening event...");
            println!("[Companion]: {:?}", line);
            if line.contains("ssh: listening on") {
                true
            } else {
                false
            }
        }

        fn parse_line_for_instance_update(line: String) -> bool {
            println!("Searhing for instance update or asking key fingerprint...");
            println!("[Companion]: {:?}", line);

            if line.contains("instance update") {
                true
            } else {
                false
            }
        }

        let config = loop {
            let line = lines.next_line().await;

            if let Ok(line) = line {
                if let Some(line) = line {
                    let config = parse_line_for_config(line);
                    break config;
                }
            }
        };

        loop {
            let line = lines.next_line().await;

            if let Ok(line) = line {
                if let Some(line) = line {
                    let is_listening = parse_line_for_listening(line);
                    if is_listening {
                        break;
                    }
                }
            }
        }

        loop {
            let line = lines.next_line().await;

            if let Ok(line) = line {
                if let Some(line) = line {
                    let is_update = parse_line_for_instance_update(line);

                    if is_update {
                        break;
                    }
                }
            }
        }

        config
    };

    if ssh_config_path.is_none() {
        println!("Ssh config not found");
        return None;
    };

    let handle = CompanionHandle {
        companion_process: companion_process,
        ssh_config_path: ssh_config_path?,
    };

    Some(handle)
}

pub async fn run_ssh(ssh_config_path: &str, instanse_id: &str, ports: Vec<u32>) -> Option<AsyncGroupChild> {
    let port_mappings = {
        let mut port_mapping = Vec::new();
        for port in ports {
            let mapping = format!("{0}:127.0.0.1:{0}", port);
            port_mapping.push("-R".to_string());
            port_mapping.push(mapping);
        }
        port_mapping
    };

    //  ssh -R 3333:127.0.0.1:3333 -F C:\Users\<user>\AppData\Local\Temp\gitpod_ssh_config  <gitpod-instanse-id>
    let mut ssh_process = Command::new("ssh")
        .arg("-o")
        .arg("StrictHostKeyChecking=no")
        .args(port_mappings)
        .arg("-F")
        .arg(ssh_config_path)
        .arg(instanse_id)
        .stderr(Stdio::piped())
        .group_spawn()
        .ok()?;

    let stderr = ssh_process.inner().stderr.take();

    let stderr = match stderr {
        Some(stderr) => stderr,
        None => {
            println!("Can not get output from ssh");
            return None;
        }
    };

    let reader = BufReader::new(stderr);
    let mut lines = reader.lines();

    loop {
        let line = lines.next_line().await;

        match line {
            Ok(line) => {
                if let Some(line) = line {
                    println!("[SSH]: {:?}", line);
                    let is_refused = line.contains("Connection refused");
                    if is_refused {
                        println!("Connection to GitPod refused");
                        return None;
                    }

                    break;
                }
            }
            Err(e) => println!("Error reading line from stderr: {}", e),
        }
    }

    Some(ssh_process)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[async_std::test]
    async fn test_run() -> std::io::Result<()> {
        let companion_handle = run_companion(HostName::Default).await;

        let companion_handle = match companion_handle {
            Some(companion_handle) => companion_handle,
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "No companion process handle created!",
                ));
            }
        };

        let ports = vec![3333];
        let gitpod_instance_id = "test-project-instance-id-on-gitpod";
        let ssh_handle = run_ssh(&companion_handle.ssh_config_path, gitpod_instance_id, ports).await;

        if let None = ssh_handle {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "No SSH process handle created!",
            ));
        };

        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        Ok(())
    }
}
