import Hls, { ErrorData, Events } from "hls.js";
import {
  createStatusPoller,
  element,
  setPhase,
  type StatusElements,
} from "./shared";

const elements: StatusElements = {
  placeholder: element<HTMLDivElement>("placeholder"),
  message: element<HTMLParagraphElement>("message"),
  retry: element<HTMLButtonElement>("retry"),
  status: element<HTMLDivElement>("status"),
  statusLabel: element<HTMLSpanElement>("status-label"),
  connection: element<HTMLSpanElement>("connection"),
  restarts: element<HTMLSpanElement>("restarts"),
  revision: element<HTMLSpanElement>("revision"),
  motionScore: element<HTMLSpanElement>("motion-score"),
  eventsEmpty: element<HTMLParagraphElement>("events-empty"),
  eventsStrip: element<HTMLDivElement>("events-strip"),
  lightbox: element<HTMLDialogElement>("lightbox"),
  lightboxImage: element<HTMLImageElement>("lightbox-image"),
  lightboxCaption: element<HTMLParagraphElement>("lightbox-caption"),
  lightboxPrev: element<HTMLButtonElement>("lightbox-prev"),
  lightboxNext: element<HTMLButtonElement>("lightbox-next"),
};

const video = element<HTMLVideoElement>("video");
const streamUrl = "/hls/stream.m3u8";

let hls: Hls | null = null;
let retryTimer: number | undefined;
let playerConnected = false;

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
  playerConnected = false;
  setPhase(elements, "starting");

  if (video.canPlayType("application/vnd.apple.mpegurl")) {
    video.src = `${streamUrl}?t=${Date.now()}`;
    video.addEventListener(
      "loadedmetadata",
      () => {
        playerConnected = true;
        void video.play().catch(() => {
          elements.connection.textContent = "Tap play to start video";
        });
      },
      { once: true },
    );
    video.addEventListener(
      "error",
      () => {
        setPhase(elements, "offline", "Playback stopped. Reconnecting…");
        scheduleReconnect();
      },
      { once: true },
    );
    return;
  }

  if (!Hls.isSupported()) {
    setPhase(elements, "offline", "This browser does not support HLS video.");
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
    playerConnected = true;
    void video.play().catch(() => {
      elements.connection.textContent = "Tap play to start video";
    });
  });
  hls.on(Events.ERROR, (_event: Events.ERROR, data: ErrorData) => {
    if (data.fatal) {
      setPhase(elements, "offline", "Playback stopped. Reconnecting…");
      hls?.destroy();
      hls = null;
      playerConnected = false;
      scheduleReconnect();
    }
  });
}

const { pollStatus } = createStatusPoller(elements, {
  liveConnectionText: "HLS over your Tailscale network",
  onLive: async () => {
    if (!playerConnected && !video.src && !hls) {
      await connectPlayer();
    }
  },
});

video.addEventListener("playing", () => setPhase(elements, "live"));
elements.retry.addEventListener("click", () => {
  void connectPlayer();
  void pollStatus();
});

void pollStatus();
