//! Servidor HTTP embutido do LegendAI (Fase 2 — "LegendAI na rede local").
//!
//! Sobe um `axum` no runtime tokio do Tauri, escutando em `0.0.0.0:<porta>`
//! (default 8765). O app GoAnime TV enfileira jobs via `POST /v1/jobs` com a URL
//! do stream e baixa o SRT pronto em `GET /v1/jobs/{id}/srt`.
//!
//! Sem autenticação (LAN confiável — decisão documentada no plano). O ponto de
//! extensão para um token futuro fica em `auth.rs` (vazio no MVP).

pub mod dto;
mod routes;

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{OnceLock, RwLock};

use serde::Serialize;
use tauri::AppHandle;

use crate::config::{AppConfig, NetConfig};

use routes::{ApiState, ServerIdentity};

/// Endereço do servidor em execução (exibido no pareamento).
#[derive(Debug, Clone, Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub host: String,
    pub port: u16,
}

static INFO: OnceLock<RwLock<Option<ServerInfo>>> = OnceLock::new();

fn info_slot() -> &'static RwLock<Option<ServerInfo>> {
    INFO.get_or_init(|| RwLock::new(None))
}

/// Nome do PC exibido no pareamento: `net.name` da config, senão o hostname.
pub fn server_name(cfg: &NetConfig) -> String {
    cfg.name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(String::from)
        .unwrap_or_else(hostname)
}

fn hostname() -> String {
    sysinfo::System::host_name()
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| "LegendAI".into())
}

/// IP local na rota padrão, descoberto sem enviar pacote (o `connect` de um
/// socket UDP só resolve a interface de saída). Fallback `127.0.0.1`.
pub fn local_ip() -> IpAddr {
    (|| {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
        socket.local_addr().ok().map(|addr| addr.ip())
    })()
    .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

/// IP do Tailscale (`100.64.0.0/10`), se houver interface ativa.
///
/// Serve para oferecer um segundo endereço/QR de pareamento quando o roteador
/// isola as redes Ethernet/Wi-Fi (client isolation/VLANs): PC e celular se
/// alcançam pelo IP `100.x` do tailnet, independente do roteador. `None` se o
/// Tailscale não estiver ativo (aí só há a rota LAN).
pub fn tailscale_ip() -> Option<IpAddr> {
    let networks = sysinfo::Networks::new_with_refreshed_list();
    networks
        .list()
        .values()
        .flat_map(|n| n.ip_networks().iter())
        .map(|ipn| ipn.addr)
        .find(is_tailscale_v4)
}

/// `100.64.0.0/10` (CGNAT — faixa usada pelo Tailscale). Não é roteável na
/// internet pública.
fn is_tailscale_v4(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 100 && (64..=127).contains(&o[1])
        }
        IpAddr::V6(_) => false,
    }
}

/// Sobe o servidor HTTP. Retorna o [`ServerInfo`] previsto (o bind em si é
/// assíncrono — falhas de porta ficam no log). O servidor fica ativo até o app
/// encerrar.
pub fn start(app: AppHandle, cfg: &NetConfig) -> Result<ServerInfo, String> {
    if cfg.port < 1024 {
        return Err(format!("porta {} inválida (use >= 1024)", cfg.port));
    }
    let identity = ServerIdentity {
        name: server_name(cfg),
        host: local_ip().to_string(),
        port: cfg.port,
        tailscale_host: tailscale_ip().map(|ip| ip.to_string()),
    };
    let info = ServerInfo {
        name: identity.name.clone(),
        host: identity.host.clone(),
        port: identity.port,
    };
    // Fase 5: remove áudios enviados por upload que ficaram órfãos (>24 h).
    routes::sweep_uploads();
    let router = routes::router(ApiState::new(app, identity));
    let logged = info.clone();
    tauri::async_runtime::spawn(async move {
        let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, logged.port));
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                *info_slot().write().unwrap() = Some(logged.clone());
                tracing::info!(
                    "servidor de rede ativo em http://{}:{} (protocolo v{})",
                    logged.host,
                    logged.port,
                    dto::PROTOCOL
                );
                if let Err(e) = axum::serve(listener, router).await {
                    tracing::error!("servidor de rede encerrou: {e}");
                    *info_slot().write().unwrap() = None;
                }
            }
            Err(e) => tracing::error!(
                "não foi possível abrir a porta {} para o app remoto: {e}",
                logged.port
            ),
        }
    });
    Ok(info)
}

/// Servidor em execução, se já subiu.
pub fn info() -> Option<ServerInfo> {
    info_slot().read().unwrap().clone()
}

/// `Some(InfoView)` quando o servidor está ativo.
pub fn info_view() -> Option<dto::InfoView> {
    info().map(|i| {
        let tailscale_host = tailscale_ip().map(|ip| ip.to_string());
        let tailscale_url = tailscale_host
            .as_ref()
            .map(|h| format!("http://{h}:{}", i.port));
        dto::InfoView {
            url: format!("http://{}:{}", i.host, i.port),
            name: i.name,
            host: i.host,
            port: i.port,
            protocol: dto::PROTOCOL,
            version: env!("CARGO_PKG_VERSION").into(),
            tailscale_host,
            tailscale_url,
        }
    })
}

/// Status do servidor para a UI (comando IPC).
#[derive(Debug, Clone, Serialize)]
pub struct NetStatus {
    pub running: bool,
    pub port: u16,
    pub url: Option<String>,
}

/// Comando IPC: dados de pareamento (IP:porta/versão) ou `None` se desligado.
#[tauri::command(rename_all = "snake_case")]
pub fn net_info() -> Option<dto::InfoView> {
    info_view()
}

/// Comando IPC: status do servidor (ligado/porta/URL).
#[tauri::command(rename_all = "snake_case")]
pub fn net_status() -> NetStatus {
    let cfg = AppConfig::load_or_default();
    let info = info();
    NetStatus {
        running: info.is_some(),
        port: cfg.net.port,
        url: info.map(|i| format!("http://{}:{}", i.host, i.port)),
    }
}

/// Comando IPC: muda a porta do servidor na config. A aplicação ocorre no
/// próximo boot (reconectar o servidor em quente exigiria derrubar o listener).
#[tauri::command(rename_all = "snake_case")]
pub fn net_set_port(port: u16) -> Result<String, String> {
    if port < 1024 {
        return Err("a porta deve ser >= 1024".into());
    }
    let mut cfg = AppConfig::load_or_default();
    cfg.net.port = port;
    cfg.save().map_err(|e| e.to_string())?;
    tracing::info!("porta do servidor de rede alterada para {port} (aplica no próximo boot)");
    Ok("porta salva — reinicie o LegendAI para aplicar".into())
}

/// Comando IPC: QR (SVG) do endereço de pareamento, para a aba "Rede".
///
/// O QR é gerado no Rust (crate `qrcode`, feature `svg`) em vez de npm: evita
/// uma dependência de build só para o frontend e o SVG já sai com o contraste
/// certo para a UI. O conteúdo é exatamente a URL `http://<ip>:<porta>` do
/// `/v1/info`, que o app GoAnime lê pela câmera (Fase 4).
#[tauri::command(rename_all = "snake_case")]
pub fn net_qr_svg() -> Result<String, String> {
    let info = info_view().ok_or_else(|| "servidor de rede não está ativo".to_string())?;
    qr_svg(&info.url)
}

/// Comando IPC: QR (SVG) de uma URL de pareamento alternativa — hoje o
/// endereço Tailscale (`http://100.x:porta`) mostrado na aba Rede. Só aceita
/// `http(s)://` para não virar um gerador de QR arbitrário.
#[tauri::command(rename_all = "snake_case")]
pub fn net_qr_svg_for(url: String) -> Result<String, String> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("URL de pareamento inválida".into());
    }
    qr_svg(trimmed)
}

/// Renderiza um texto como QR em SVG. Público para reuso/testes.
pub fn qr_svg(payload: &str) -> Result<String, String> {
    let code = qrcode::QrCode::new(payload.as_bytes()).map_err(|e| e.to_string())?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_svg_gera_svg_valido() {
        let svg = qr_svg("http://192.168.2.109:8765").expect("qr");
        assert!(svg.contains("<svg"), "saída deve ser SVG: {svg}");
        assert!(svg.contains("path"), "QR deve ter módulos desenhados");
        assert!(svg.len() > 200, "SVG pequeno demais: {} bytes", svg.len());
    }

    #[test]
    fn qr_svg_aceita_url_legendai() {
        // O app aceita tanto http quanto o esquema legendai:// (Fase 4).
        assert!(qr_svg("legendai://v1/pair?host=192.168.2.109&port=8765").is_ok());
    }

    #[test]
    fn tailscale_v4_reconhece_a_faixa_cgnat() {
        assert!(is_tailscale_v4(&IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(is_tailscale_v4(&IpAddr::V4(Ipv4Addr::new(
            100, 127, 255, 254
        ))));
        // Bordas fora da faixa 100.64.0.0/10.
        assert!(!is_tailscale_v4(&IpAddr::V4(Ipv4Addr::new(100, 63, 0, 1))));
        assert!(!is_tailscale_v4(&IpAddr::V4(Ipv4Addr::new(100, 128, 0, 1))));
        assert!(!is_tailscale_v4(&IpAddr::V4(Ipv4Addr::new(
            192, 168, 2, 109
        ))));
    }

    #[test]
    fn qr_for_aceita_http_e_rejeita_outros_esquemas() {
        assert!(net_qr_svg_for("http://100.125.210.81:8765".into()).is_ok());
        assert!(net_qr_svg_for("https://pc.tailnet.ts.net".into()).is_ok());
        assert!(net_qr_svg_for("ftp://x".into()).is_err());
        assert!(net_qr_svg_for("nada".into()).is_err());
    }
}
