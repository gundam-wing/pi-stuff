import Hls, { ErrorData, Events } from "hls.js";
import "./style.css";

type Phase = "starting" | "live" | "offline";

interface StreamStatus {
  phase: Phase;
  message: string | null;
  startedAt: number | null;
  restarts: number;
}

const video = element<HTMLVideoElement>("video");
const placeholder = element<HTMLDivElement>("placeholder");
const message = element<HTMLParagraphElement>("message");
const retry = element<HTMLButtonElement>("retry");
const status = element<HTMLDivElement>("status");
const statusLabel = element<HTMLSpanElement>("status-label");
const connection = element<HTMLSpanElement>("connection");
const restarts = element<HTMLSpanElement>("restarts");
const streamUrl = "/hls/stream.m3u8";

let hls: Hls | null = null;
let pollTimer: number | undefined;
let retryTimer: number | undefined;

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) {
    throw new Error(`Missing required element #${id}`);
  }
  return found as T;
}

function setPhase(phase: Phase, detail?: string): void {
  status.dataset.phase = phase;
  statusLabel.textContent =
    phase === "live" ? "Live" : phase === "offline" ? "Offline" : "Connecting";
  message.textContent =
    detail ??
    (phase === "offline"
      ? "The camera is currently unavailable."
      : "Waiting for the camera stream…");
  placeholder.hidden = phase === "live";
  retry.hidden = phase !== "offline";
}

function scheduleReconnect(delay = 3_000): void {
  window.clearTimeout(retryTimer);
  retryTimer = window.setTimeout(() => {
    void connectPlayer();
  }, delay);
}

async function connectPlayer(): Promise<void> {
  window.clearTimeout(retryTimer);
  hls?.destroy();
  hls = null;
  video.removeAttribute("src");
  video.load();
  setPhase("starting");

  if (video.canPlayType("application/vnd.apple.mpegurl")) {
    video.src = `${streamUrl}?t=${Date.now()}`;
    video.addEventListener(
      "loadedmetadata",
      () => {
        void video.play().catch(() => {
          connection.textContent = "Tap play to start video";
        });
      },
      { once: true },
    );
    video.addEventListener(
      "error",
      () => {
        setPhase("offline", "Playback stopped. Reconnecting…");
        scheduleReconnect();
      },
      { once: true },
    );
    return;
  }

  if (!Hls.isSupported()) {
    setPhase("offline", "This browser does not support HLS video.");
    return;
  }

  hls = new Hls({
    lowLatencyMode: false,
    liveSyncDurationCount: 2,
    liveMaxLatencyDurationCount: 5,
    maxBufferLength: 12,
  });
  hls.loadSource(streamUrl);
  hls.attachMedia(video);
  hls.on(Events.MANIFEST_PARSED, () => {
    void video.play().catch(() => {
      connection.textContent = "Tap play to start video";
    });
  });
  hls.on(Events.ERROR, (_event: Events.ERROR, data: ErrorData) => {
    if (data.fatal) {
      setPhase("offline", "Playback stopped. Reconnecting…");
      hls?.destroy();
      hls = null;
      scheduleReconnect();
    }
  });
}

async function pollStatus(): Promise<void> {
  try {
    const response = await fetch("/api/status", { cache: "no-store" });
    if (!response.ok) {
      throw new Error(`status returned ${response.status}`);
    }
    const current = (await response.json()) as StreamStatus;
    setPhase(current.phase, current.message ?? undefined);
    connection.textContent =
      current.phase === "live"
        ? "Encrypted over your Tailscale network"
        : "Waiting for the Pi";
    restarts.textContent =
      current.restarts > 0
        ? `${current.restarts} pipeline restart${current.restarts === 1 ? "" : "s"}`
        : "";

    if (current.phase === "live" && !video.src && !hls) {
      await connectPlayer();
    }
  } catch {
    setPhase("offline", "The Pi monitor could not be reached.");
    connection.textContent = "Check Tailscale and the Pi";
  } finally {
    window.clearTimeout(pollTimer);
    pollTimer = window.setTimeout(() => {
      void pollStatus();
    }, 3_000);
  }
}

video.addEventListener("playing", () => setPhase("live"));
retry.addEventListener("click", () => {
  void connectPlayer();
  void pollStatus();
});
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) {
    void pollStatus();
  }
});

void pollStatus();
