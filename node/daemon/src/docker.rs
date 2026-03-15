use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions, LogOutput,
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

// ── Public container summary ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ContainerInfo {
    pub id: String,
    pub image: String,
    pub status: String,
}

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
/// Maps `host_port` → container port 7001 and mounts `data_volume` at /data.
/// Returns the Docker container ID.
pub async fn start_capability(
    image: &str,
    host_port: u16,
    data_volume: PathBuf,
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

    // Port binding: host_port → 7001/tcp inside the container
    let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
    port_bindings.insert(
        "7001/tcp".to_string(),
        Some(vec![PortBinding {
            host_ip: Some("0.0.0.0".to_string()),
            host_port: Some(host_port.to_string()),
        }]),
    );

    let host_config = HostConfig {
        port_bindings: Some(port_bindings),
        binds: Some(vec![format!("{}:/data", data_dir_str)]),
        ..Default::default()
    };

    let create_options = CreateContainerOptions {
        name: container_name.as_str(),
        platform: None,
    };

    let config = ContainerConfig {
        image: Some(image),
        host_config: Some(host_config),
        ..Default::default()
    };

    let response = docker
        .create_container(Some(create_options), config)
        .await?;
    docker
        .start_container(&response.id, None::<StartContainerOptions<String>>)
        .await?;

    info!(
        "Started container {} (name={}) for image {}",
        response.id, container_name, image
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
                        // Log stderr from the exec but don't treat it as content
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
    let running = info
        .state
        .and_then(|s| s.running)
        .unwrap_or(false);
    Ok(running)
}

pub async fn list_running() -> anyhow::Result<Vec<ContainerInfo>> {
    let docker = connect()?;
    let containers = docker.list_containers::<String>(None).await?;
    let infos = containers
        .into_iter()
        .filter_map(|c| {
            let id = c.id?;
            let image = c.image.unwrap_or_default();
            let status = c.status.unwrap_or_default();
            Some(ContainerInfo { id, image, status })
        })
        .collect();
    Ok(infos)
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
