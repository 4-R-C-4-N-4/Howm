use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, LogOutput, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::image::CreateImageOptions;
use bollard::models::{HostConfig, PortBinding};
use bollard::Docker;
use futures_util::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::info;

// ── Connection helper ────────────────────────────────────────────────────────

pub fn connect() -> anyhow::Result<Docker> {
    Docker::connect_with_local_defaults()
        .map_err(|e| anyhow::anyhow!("Docker connect failed: {}", e))
}

// ── Image operations ─────────────────────────────────────────────────────────

pub async fn pull_image(image: &str) -> anyhow::Result<()> {
    let docker = connect()?;
    info!("Pulling image: {}", image);

    let options = CreateImageOptions {
        from_image: image,
        ..Default::default()
    };

    let mut stream = docker.create_image(Some(options), None, None);
    while let Some(result) = stream.next().await {
        match result {
            Ok(info) => {
                if let Some(status) = info.status {
                    info!("Pull: {}", status);
                }
            }
            Err(e) => return Err(anyhow::anyhow!("Pull failed: {}", e)),
        }
    }
    Ok(())
}

// ── Container lifecycle ──────────────────────────────────────────────────────

/// Start a capability container.
/// Maps `host_port` → `container_port` (from manifest, default 7001).
/// Applies resource limits from the manifest.
/// Returns the Docker container ID.
pub async fn start_capability(
    image: &str,
    host_port: u16,
    data_volume: PathBuf,
    container_port: u16,
    resources: Option<&ResourcesManifest>,
) -> anyhow::Result<String> {
    let docker = connect()?;

    let short_id = uuid::Uuid::new_v4()
        .to_string()
        .split('-')
        .next()
        .unwrap_or("cap")
        .to_string();
    let container_name = format!("howm-cap-{}", short_id);
    let data_dir_str = data_volume.to_string_lossy().to_string();

    // Port binding: host_port → container_port/tcp
    let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
    port_bindings.insert(
        format!("{}/tcp", container_port),
        Some(vec![PortBinding {
            host_ip: Some("127.0.0.1".to_string()), // S1: only bind locally
            host_port: Some(host_port.to_string()),
        }]),
    );

    // S6: Apply resource limits
    let (memory_limit, nano_cpus) = if let Some(res) = resources {
        let mem = parse_memory_limit(res.memory.as_deref()).unwrap_or(256 * 1024 * 1024);
        let cpu = parse_cpu_limit(res.cpu.as_deref()).unwrap_or(500_000_000);
        (Some(mem), Some(cpu))
    } else {
        (Some(256 * 1024 * 1024), Some(500_000_000)) // 256 MB, 0.5 CPU default
    };

    let host_config = HostConfig {
        port_bindings: Some(port_bindings),
        binds: Some(vec![format!("{}:/data", data_dir_str)]),
        memory: memory_limit,
        nano_cpus,
        readonly_rootfs: Some(true),
        security_opt: Some(vec!["no-new-privileges:true".to_string()]),
        ..Default::default()
    };

    let create_options = CreateContainerOptions {
        name: container_name.as_str(),
        platform: None,
    };

    // Set PORT env var so the capability knows which port to listen on
    let env_vars = [format!("PORT={}", container_port)];

    let config = ContainerConfig {
        image: Some(image),
        host_config: Some(host_config),
        env: Some(env_vars.iter().map(|s| s.as_str()).collect()),
        ..Default::default()
    };

    let response = docker
        .create_container(Some(create_options), config)
        .await?;
    docker
        .start_container(&response.id, None::<StartContainerOptions<String>>)
        .await?;

    info!(
        "Started container {} (name={}) for image {} [port {}→{}, mem={}MB, cpu={}m]",
        response.id,
        container_name,
        image,
        host_port,
        container_port,
        memory_limit.unwrap_or(0) / 1024 / 1024,
        nano_cpus.unwrap_or(0) / 1_000_000,
    );
    Ok(response.id)
}

pub async fn stop_capability(container_id: &str) -> anyhow::Result<()> {
    let docker = connect()?;
    docker
        .stop_container(container_id, Some(StopContainerOptions { t: 10 }))
        .await
        .map_err(|e| anyhow::anyhow!("Stop failed: {}", e))?;
    Ok(())
}

pub async fn remove_container(container_id: &str) -> anyhow::Result<()> {
    let docker = connect()?;
    let options = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };
    docker
        .remove_container(container_id, Some(options))
        .await
        .map_err(|e| anyhow::anyhow!("Remove failed: {}", e))?;
    Ok(())
}

// ── Manifest reading via exec ────────────────────────────────────────────────

/// Read /capability.yaml from inside a running container and parse it.
pub async fn read_manifest(container_id: &str) -> anyhow::Result<CapabilityManifest> {
    let docker = connect()?;

    let exec_id = docker
        .create_exec(
            container_id,
            CreateExecOptions {
                cmd: Some(vec!["cat", "/capability.yaml"]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await?
        .id;

    let mut content = String::new();

    match docker.start_exec(&exec_id, None).await? {
        StartExecResults::Attached { mut output, .. } => {
            while let Some(chunk) = output.next().await {
                match chunk? {
                    LogOutput::StdOut { message } => {
                        content.push_str(&String::from_utf8_lossy(&message));
                    }
                    LogOutput::StdErr { message } => {
                        let text = String::from_utf8_lossy(&message);
                        tracing::warn!("read_manifest stderr: {}", text);
                    }
                    _ => {}
                }
            }
        }
        StartExecResults::Detached => {
            return Err(anyhow::anyhow!("exec detached unexpectedly"));
        }
    }

    if content.is_empty() {
        return Err(anyhow::anyhow!(
            "capability.yaml is empty or not found in container {}",
            container_id
        ));
    }

    let manifest: CapabilityManifest = serde_yaml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse capability.yaml: {}", e))?;

    Ok(manifest)
}

// ── Health / listing ─────────────────────────────────────────────────────────

pub async fn check_health(container_id: &str) -> anyhow::Result<bool> {
    let docker = connect()?;
    let info = docker.inspect_container(container_id, None).await?;
    let running = info.state.and_then(|s| s.running).unwrap_or(false);
    Ok(running)
}

// ── Capability manifest types ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub api: Option<ApiManifest>,
    pub discovery: Option<DiscoveryManifest>,
    pub permissions: Option<PermissionsManifest>,
    pub resources: Option<ResourcesManifest>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiManifest {
    pub base_path: Option<String>,
    pub endpoints: Option<Vec<EndpointManifest>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointManifest {
    pub name: String,
    pub method: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryManifest {
    pub advertise: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionsManifest {
    pub visibility: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcesManifest {
    pub cpu: Option<String>,
    pub memory: Option<String>,
}

// ── Resource limit parsers ───────────────────────────────────────────────────

/// Parse memory limit string like "256M", "1G", "512Mi" into bytes.
fn parse_memory_limit(s: Option<&str>) -> Option<i64> {
    let s = s?;
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let (num_str, multiplier) = if s.ends_with("Gi") || s.ends_with("G") {
        let n = s.trim_end_matches("Gi").trim_end_matches("G");
        (n, 1024 * 1024 * 1024i64)
    } else if s.ends_with("Mi") || s.ends_with("M") {
        let n = s.trim_end_matches("Mi").trim_end_matches("M");
        (n, 1024 * 1024i64)
    } else if s.ends_with("Ki") || s.ends_with("K") {
        let n = s.trim_end_matches("Ki").trim_end_matches("K");
        (n, 1024i64)
    } else {
        (s, 1i64) // raw bytes
    };

    num_str.trim().parse::<i64>().ok().map(|n| n * multiplier)
}

/// Parse CPU limit string like "0.5", "500m", "1" into nano CPUs.
fn parse_cpu_limit(s: Option<&str>) -> Option<i64> {
    let s = s?;
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    if s.ends_with("m") {
        // millicores → nanocores
        let n = s.trim_end_matches("m");
        n.parse::<i64>().ok().map(|v| v * 1_000_000)
    } else {
        // fractional cores → nanocores
        s.parse::<f64>().ok().map(|v| (v * 1_000_000_000.0) as i64)
    }
}
