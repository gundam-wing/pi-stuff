use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast};
use tracing::{info, warn};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

#[derive(Clone)]
pub(crate) struct WebRtcHub {
    nal_tx: broadcast::Sender<Bytes>,
    frame_duration: Duration,
    active_sessions: Arc<AtomicU64>,
    api: Arc<webrtc::api::API>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WebRtcStatus {
    active_sessions: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SdpMessage {
    #[serde(rename = "type")]
    sdp_type: String,
    sdp: String,
}

impl WebRtcHub {
    pub(crate) fn new(frame_rate: u32) -> Result<Self> {
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .context("register WebRTC codecs")?;
        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut media_engine)
            .context("register WebRTC interceptors")?;
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        Ok(Self {
            nal_tx: broadcast::channel(64).0,
            frame_duration: Duration::from_secs_f64(1.0 / f64::from(frame_rate.max(1))),
            active_sessions: Arc::new(AtomicU64::new(0)),
            api: Arc::new(api),
        })
    }

    pub(crate) fn publish_nal(&self, data: Bytes) {
        let _ = self.nal_tx.send(data);
    }

    pub(crate) fn status(&self) -> WebRtcStatus {
        WebRtcStatus {
            active_sessions: self.active_sessions.load(Ordering::Relaxed),
        }
    }

    pub(crate) async fn handle_offer(&self, offer: SdpMessage) -> Result<SdpMessage> {
        if offer.sdp_type != "offer" {
            anyhow::bail!("expected SDP offer, got {}", offer.sdp_type);
        }

        let peer_connection = Arc::new(
            self.api
                .new_peer_connection(RTCConfiguration {
                    ice_servers: vec![RTCIceServer {
                        urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                        ..Default::default()
                    }],
                    ..Default::default()
                })
                .await
                .context("create WebRTC peer connection")?,
        );

        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_owned(),
                ..Default::default()
            },
            "video".to_owned(),
            "pi-camera-monitor".to_owned(),
        ));

        let rtp_sender = peer_connection
            .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await
            .context("add WebRTC video track")?;

        tokio::spawn(async move {
            let mut rtcp_buf = vec![0u8; 1500];
            while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {}
        });

        let nal_rx = self.nal_tx.subscribe();
        let frame_duration = self.frame_duration;
        let active_sessions = self.active_sessions.clone();
        let connected = Arc::new(RwLock::new(false));

        let connected_for_ice = connected.clone();
        peer_connection.on_ice_connection_state_change(Box::new(
            move |state: RTCIceConnectionState| {
                let connected = connected_for_ice.clone();
                Box::pin(async move {
                    let is_connected = state == RTCIceConnectionState::Connected;
                    *connected.write().await = is_connected;
                    if is_connected {
                        info!("WebRTC viewer connected");
                    }
                })
            },
        ));

        let active_for_state = active_sessions.clone();
        let peer_for_state = peer_connection.clone();
        peer_connection.on_peer_connection_state_change(Box::new(
            move |state: RTCPeerConnectionState| {
                let active_for_state = active_for_state.clone();
                let peer_for_state = peer_for_state.clone();
                Box::pin(async move {
                    match state {
                        RTCPeerConnectionState::Connected => {
                            active_for_state.fetch_add(1, Ordering::Relaxed);
                        }
                        RTCPeerConnectionState::Closed | RTCPeerConnectionState::Failed => {
                            active_for_state.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                                count.checked_sub(1)
                            }).ok();
                            let _ = peer_for_state.close().await;
                        }
                        _ => {}
                    }
                })
            },
        ));

        tokio::spawn(async move {
            let mut nal_rx = nal_rx;
            loop {
                match nal_rx.recv().await {
                    Ok(nal) => {
                        if !*connected.read().await {
                            continue;
                        }
                        if let Err(error) = video_track
                            .write_sample(&Sample {
                                data: nal,
                                duration: frame_duration,
                                ..Default::default()
                            })
                            .await
                        {
                            warn!(%error, "failed to write WebRTC sample");
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        });

        peer_connection
            .set_remote_description(RTCSessionDescription::offer(offer.sdp)?)
            .await
            .context("set remote WebRTC description")?;

        let answer = peer_connection
            .create_answer(None)
            .await
            .context("create WebRTC answer")?;
        let mut gather_complete = peer_connection.gathering_complete_promise().await;
        peer_connection
            .set_local_description(answer)
            .await
            .context("set local WebRTC description")?;
        let _ = gather_complete.recv().await;

        let local_description = peer_connection
            .local_description()
            .await
            .context("missing local WebRTC description")?;

        Ok(SdpMessage {
            sdp_type: local_description.sdp_type.to_string(),
            sdp: local_description.sdp,
        })
    }
}

use crate::AppState;

pub(crate) async fn webrtc_status(State(state): State<AppState>) -> Json<WebRtcStatus> {
    Json(state.webrtc.status())
}

pub(crate) async fn webrtc_offer(
    State(state): State<AppState>,
    Json(offer): Json<SdpMessage>,
) -> Response {
    match state.webrtc.handle_offer(offer).await {
        Ok(answer) => Json(answer).into_response(),
        Err(error) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}
