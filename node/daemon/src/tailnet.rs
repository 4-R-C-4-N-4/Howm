use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, StartContainerOptions,
    StopContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::{DeviceMapping, HostConfig, PortBinding};
use bollard::Docker;
use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{info, warn};

const HEADSCALE_CONTAINER: &str = "howm-headscale";
const TAILSCALE_CONTAINER: &str = "howm-tailscale";
const HEADSCALE_IMAGE: &str = "headscale/headscale:latest";
const TAILSCALE_IMAGE: &str = "tailscale/tailscale:latest";

// ── Public types ──────────────────────────────────────────────────────────────

pub struct TailnetState {
    pub ip: Option<String>,
    pub name: Option<String>,
    pub status: String,
    pub headscale_container_id: Option<String>,
    pub tailscale_container_id: Option<String>,
}

pub struct TailnetConfig {
    pub enabled: bool,
    pub coordination_url: Option<String>,
    pub authkey: Option<String>,
    pub data_dir: PathBuf,
    pub headscale_enabled: bool,
    pub headscale_port: u16,
}

// ── Entry points ──────────────────────────────────────────────────────────────

pub async fn init(config: TailnetConfig) -> anyhow::Result<TailnetState> {
    if !config.enabled {
        info!("Tailnet disabled");
        return Ok(TailnetState {
            ip: None,
            name: None,
            status: "disabled".to_string(),
            headscale_container_id: None,
            tailscale_container_id: None,
        });
    }

    let docker = match crate::docker::connect() {
        Ok(d) => d,
        Err(e) => {
            warn!("Docker not available, tailnet disabled: {}", e);
            return Ok(TailnetState {
                ip: None,
                name: None,
                status: "docker-unavailable".to_string(),
                headscale_container_id: None,
                tailscale_container_id: None,
            });
        }
    };

    // --- Pull images ----------------------------------------------------------
    info!("Pulling tailnet images...");
    pull_image(&docker, TAILSCALE_IMAGE).await?;

    // --- Headscale (optional) -------------------------------------------------
    let mut headscale_container_id: Option<String> = None;
    let coordination_url: String;

    if config.headscale_enabled {
        pull_image(&docker, HEADSCALE_IMAGE).await?;
        let id = ensure_headscale(&docker, &config).await?;
        coordination_url = format!("http://127.0.0.1:{}", config.headscale_port);
        headscale_container_id = Some(id);
    } else {
        coordination_url = config.coordination_url.clone().unwrap_or_default();
    }

    // --- Auth key -------------------------------------------------------------
    let authkey: String = if config.headscale_enabled {
        match create_headscale_authkey(&docker).await {
            Ok(key) => {
                info!("Generated headscale auth key");
                key
            }
            Err(e) => {
                warn!("Failed to generate headscale auth key: {}. Proceeding without key.", e);
                String::new()
            }
        }
    } else {
        config.authkey.clone().unwrap_or_default()
    };

    // --- Start tailscale container --------------------------------------------
    let ts_id = ensure_tailscale(&docker, &config, &coordination_url, &authkey).await?;

    // --- Wait for IP assignment (up to 60 s) ----------------------------------
    info!("Waiting for tailnet IP assignment...");
    let mut ip: Option<String> = None;
    let mut tailnet_name: Option<String> = None;

    for attempt in 0..30 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        match get_tailscale_status(&docker, &ts_id).await {
            Ok(Some((assigned_ip, name))) => {
                ip = Some(assigned_ip);
                tailnet_name = Some(name);
                break;
            }
            Ok(None) => {
                if attempt % 5 == 0 {
                    info!("Tailnet IP not yet assigned (attempt {})", attempt + 1);
                }
            }
            Err(e) => {
                if attempt % 5 == 0 {
                    warn!("Error checking tailnet status: {}", e);
                }
            }
        }
    }

    if ip.is_none() {
        warn!("Tailnet IP not assigned after 60 s — continuing without tailnet IP");
    } else {
        info!("Tailnet connected: {:?} ({:?})", ip, tailnet_name);
    }

    Ok(TailnetState {
        status: if ip.is_some() {
            "connected".to_string()
        } else {
            "pending".to_string()
        },
        ip,
        name: tailnet_name,
        headscale_container_id,
        tailscale_container_id: Some(ts_id),
    })
}

pub async fn shutdown(state: &TailnetState) -> anyhow::Result<()> {
    let docker = match crate::docker::connect() {
        Ok(d) => d,
        Err(_) => return Ok(()),
    };

    if let Some(ref id) = state.tailscale_container_id {
        info!("Stopping tailscale container {}...", id);
        let _ = docker
            .stop_container(id, Some(StopContainerOptions { t: 10 }))
            .await;
    }
    if let Some(ref id) = state.headscale_container_id {
        info!("Stopping headscale container {}...", id);
        let _ = docker
            .stop_container(id, Some(StopContainerOptions { t: 10 }))
            .await;
    }
    Ok(())
}

// ── Headscale ────────────────────────────────────────────────────────────────

async fn ensure_headscale(docker: &Docker, config: &TailnetConfig) -> anyhow::Result<String> {
    // Reuse if already running
    if let Some(id) = find_container(docker, HEADSCALE_CONTAINER).await? {
        info!("Reusing existing headscale container {}", id);
        return Ok(id);
    }

    // Prepare config and data directories on the host
    let hs_config_dir = config.data_dir.join("headscale");
    let hs_data_dir = config.data_dir.join("headscale-data");
    std::fs::create_dir_all(&hs_config_dir)?;
    std::fs::create_dir_all(&hs_data_dir)?;

    let config_content = include_str!("../../../infra/docker/headscale/config.yaml")
        .replace("{{HEADSCALE_PORT}}", &config.headscale_port.to_string());
    std::fs::write(hs_config_dir.join("config.yaml"), &config_content)?;

    // Port binding: headscale_port on host → 8080 in container
    let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
    port_bindings.insert(
        "8080/tcp".to_string(),
        Some(vec![PortBinding {
            host_ip: Some("127.0.0.1".to_string()),
            host_port: Some(config.headscale_port.to_string()),
        }]),
    );

    let host_config = HostConfig {
        port_bindings: Some(port_bindings),
        binds: Some(vec![
            format!(
                "{}:/etc/headscale",
                hs_config_dir.to_string_lossy()
            ),
            format!(
                "{}:/var/lib/headscale",
                hs_data_dir.to_string_lossy()
            ),
        ]),
        ..Default::default()
    };

    let resp = docker
        .create_container(
            Some(CreateContainerOptions {
                name: HEADSCALE_CONTAINER,
                platform: None,
            }),
            ContainerConfig {
                image: Some(HEADSCALE_IMAGE),
                cmd: Some(vec!["headscale", "serve"]),
                host_config: Some(host_config),
                ..Default::default()
            },
        )
        .await?;

    docker
        .start_container(&resp.id, None::<StartContainerOptions<String>>)
        .await?;
    info!("Started headscale container {}", resp.id);

    // Wait for the process to initialise (max ~30 s)
    for _ in 0..15 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if exec_in_container(docker, &resp.id, &["headscale", "version"])
            .await
            .is_ok()
        {
            info!("Headscale is ready");
            break;
        }
    }

    Ok(resp.id)
}

async fn create_headscale_authkey(docker: &Docker) -> anyhow::Result<String> {
    // headscale >= 0.23 uses "users" (previously "namespaces")
    let _ = exec_in_container(
        docker,
        HEADSCALE_CONTAINER,
        &["headscale", "users", "create", "howm"],
    )
    .await;

    let output = exec_in_container(
        docker,
        HEADSCALE_CONTAINER,
        &[
            "headscale",
            "authkeys",
            "generate",
            "--reusable",
            "--expiration",
            "8760h",
            "--user",
            "howm",
        ],
    )
    .await?;

    // The key is the last non-empty line of the output
    let key = output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .last()
        .map(|l| l.trim().to_string())
        .ok_or_else(|| {
            anyhow::anyhow!("No auth key in headscale output: {}", output)
        })?;

    Ok(key)
}

// ── Tailscale ────────────────────────────────────────────────────────────────

async fn ensure_tailscale(
    docker: &Docker,
    config: &TailnetConfig,
    coordination_url: &str,
    authkey: &str,
) -> anyhow::Result<String> {
    // Reuse if already running
    if let Some(id) = find_container(docker, TAILSCALE_CONTAINER).await? {
        info!("Reusing existing tailscale container {}", id);
        return Ok(id);
    }

    let ts_state_dir = config.data_dir.join("tailscale");
    std::fs::create_dir_all(&ts_state_dir)?;

    // Build environment variables for the container
    let mut env: Vec<String> = vec![
        "TS_STATE_DIR=/var/lib/tailscale".to_string(),
        "TS_HOSTNAME=howm-node".to_string(),
    ];

    if !authkey.is_empty() {
        env.push(format!("TS_AUTHKEY={}", authkey));
    }

    // Build TS_EXTRA_ARGS
    let mut extra_args: Vec<String> = Vec::new();
    if !coordination_url.is_empty() {
        extra_args.push(format!("--login-server={}", coordination_url));
    }
    extra_args.push("--accept-routes".to_string());
    env.push(format!("TS_EXTRA_ARGS={}", extra_args.join(" ")));

    // Platform-specific host configuration
    let is_linux = cfg!(target_os = "linux");

    let host_config = if is_linux {
        info!("Using host networking for tailscale container (Linux)");
        HostConfig {
            network_mode: Some("host".to_string()),
            cap_add: Some(vec!["NET_ADMIN".to_string(), "SYS_MODULE".to_string()]),
            devices: Some(vec![DeviceMapping {
                path_on_host: Some("/dev/net/tun".to_string()),
                path_in_container: Some("/dev/net/tun".to_string()),
                cgroup_permissions: Some("rwm".to_string()),
            }]),
            binds: Some(vec![format!(
                "{}:/var/lib/tailscale",
                ts_state_dir.to_string_lossy()
            )]),
            ..Default::default()
        }
    } else {
        // macOS / Windows (Docker Desktop): userspace networking
        info!("Using userspace networking for tailscale container (non-Linux host)");
        env.push("TS_USERSPACE=1".to_string());
        HostConfig {
            binds: Some(vec![format!(
                "{}:/var/lib/tailscale",
                ts_state_dir.to_string_lossy()
            )]),
            ..Default::default()
        }
    };

    let env_refs: Vec<&str> = env.iter().map(|s| s.as_str()).collect();

    let resp = docker
        .create_container(
            Some(CreateContainerOptions {
                name: TAILSCALE_CONTAINER,
                platform: None,
            }),
            ContainerConfig {
                image: Some(TAILSCALE_IMAGE),
                env: Some(env_refs),
                host_config: Some(host_config),
                ..Default::default()
            },
        )
        .await?;

    docker
        .start_container(&resp.id, None::<StartContainerOptions<String>>)
        .await?;
    info!("Started tailscale container {}", resp.id);
    Ok(resp.id)
}

/// Run `tailscale status --json` inside the container and extract the tailnet
/// IPv4 address and DNS name. Returns `None` if tailscaled is not yet
/// authenticated / assigned.
async fn get_tailscale_status(
    docker: &Docker,
    container_id: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let output =
        exec_in_container(docker, container_id, &["tailscale", "status", "--json"]).await?;

    let json: serde_json::Value = serde_json::from_str(output.trim())?;

    // Prefer IPv4 (no colon), fall back to IPv6
    let ip = json["Self"]["TailscaleIPs"]
        .as_array()
        .and_then(|ips| {
            ips.iter()
                .find(|v| v.as_str().map(|s| !s.contains(':')).unwrap_or(false))
                .or_else(|| ips.first())
        })
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let name = json["Self"]["DNSName"]
        .as_str()
        .map(|s| s.trim_end_matches('.').to_string())
        .or_else(|| {
            json["Self"]["HostName"]
                .as_str()
                .map(|s| s.to_string())
        });

    match ip {
        Some(ip) if !ip.is_empty() => Ok(Some((ip, name.unwrap_or_default()))),
        _ => Ok(None),
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Return the container ID if a container with the given *name* is currently
/// running (status "running"), otherwise `None`.
async fn find_container(
    docker: &Docker,
    name: &str,
) -> anyhow::Result<Option<String>> {
    use bollard::container::ListContainersOptions;
    let opts = ListContainersOptions::<String> {
        all: false, // only running containers
        ..Default::default()
    };
    let containers = docker.list_containers(Some(opts)).await?;
    for c in containers {
        let names = c.names.unwrap_or_default();
        if names
            .iter()
            .any(|n| n.trim_start_matches('/') == name)
        {
            if let Some(id) = c.id {
                return Ok(Some(id));
            }
        }
    }
    Ok(None)
}

/// Pull a Docker image, streaming progress to the log.
async fn pull_image(docker: &Docker, image: &str) -> anyhow::Result<()> {
    use bollard::image::CreateImageOptions;
    info!("Pulling image: {}", image);
    let mut stream = docker.create_image(
        Some(CreateImageOptions {
            from_image: image,
            ..Default::default()
        }),
        None,
        None,
    );
    while let Some(result) = stream.next().await {
        match result {
            Ok(info) => {
                if let Some(s) = info.status {
                    info!("Pull [{}]: {}", image, s);
                }
            }
            Err(e) => return Err(anyhow::anyhow!("Pull failed for {}: {}", image, e)),
        }
    }
    Ok(())
}

/// Run a command inside a container via `docker exec` and return combined
/// stdout + stderr as a `String`.
async fn exec_in_container(
    docker: &Docker,
    container: &str,
    cmd: &[&str],
) -> anyhow::Result<String> {
    use bollard::container::LogOutput;

    let exec_id = docker
        .create_exec(
            container,
            CreateExecOptions {
                cmd: Some(cmd.to_vec()),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await?
        .id;

    let mut output = String::new();

    match docker.start_exec(&exec_id, None).await? {
        StartExecResults::Attached {
            output: mut stream, ..
        } => {
            while let Some(chunk) = stream.next().await {
                match chunk? {
                    LogOutput::StdOut { message } => {
                        output.push_str(&String::from_utf8_lossy(&message));
                    }
                    LogOutput::StdErr { message } => {
                        output.push_str(&String::from_utf8_lossy(&message));
                    }
                    _ => {}
                }
            }
        }
        StartExecResults::Detached => {}
    }

    Ok(output)
}
