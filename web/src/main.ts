import Hls, { ErrorData, Events } from "hls.js";
import "./style.css";

type Phase = "starting" | "live" | "offline";

interface MotionStatus {
  score: number;
  threshold: number;
  detecting: boolean;
}

interface MotionEvent {
  id: string;
  capturedAt: number;
  frames: number;
  score: number;
}

interface StreamStatus {
  phase: Phase;
  message: string | null;
  startedAt: number | null;
  restarts: number;
  revision: string;
  motion: MotionStatus;
  events: MotionEvent[];
}

const video = element<HTMLVideoElement>("video");
const placeholder = element<HTMLDivElement>("placeholder");
const message = element<HTMLParagraphElement>("message");
const retry = element<HTMLButtonElement>("retry");
const status = element<HTMLDivElement>("status");
const statusLabel = element<HTMLSpanElement>("status-label");
const connection = element<HTMLSpanElement>("connection");
const restarts = element<HTMLSpanElement>("restarts");
const revision = element<HTMLSpanElement>("revision");
const motionScore = element<HTMLSpanElement>("motion-score");
const eventsEmpty = element<HTMLParagraphElement>("events-empty");
const eventsStrip = element<HTMLDivElement>("events-strip");
const lightbox = element<HTMLDialogElement>("lightbox");
const lightboxImage = element<HTMLImageElement>("lightbox-image");
const lightboxCaption = element<HTMLParagraphElement>("lightbox-caption");
const lightboxPrev = element<HTMLButtonElement>("lightbox-prev");
const lightboxNext = element<HTMLButtonElement>("lightbox-next");
const streamUrl = "/hls/stream.m3u8";
const eventTime = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  second: "2-digit",
});

let hls: Hls | null = null;
let pollTimer: number | undefined;
let retryTimer: number | undefined;
let events: MotionEvent[] = [];
let renderedEventKey = "";
let lightboxEvent: MotionEvent | null = null;
let lightboxFrame = 0;

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

function percent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}

function frameSrc(id: string, frame: number): string {
  return `/events/${id}/${String(frame).padStart(2, "0")}.jpg`;
}

function renderMotion(current: StreamStatus): void {
  if (current.motion.detecting) {
    motionScore.textContent = `${percent(current.motion.score)} / ${percent(current.motion.threshold)}`;
    motionScore.title = "Latest motion score versus the capture threshold";
  } else {
    motionScore.textContent = "Waiting";
    motionScore.title = "Motion analysis starts a few seconds after the stream is live";
  }

  events = current.events;
  const key = events.map((event) => event.id).join(",");
  if (key === renderedEventKey) {
    return;
  }
  renderedEventKey = key;
  eventsEmpty.hidden = events.length > 0;
  eventsStrip.hidden = events.length === 0;
  eventsStrip.replaceChildren();
  for (const event of events) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "event-thumb";
    const captured = eventTime.format(new Date(event.capturedAt));
    button.title = captured;
    button.setAttribute("aria-label", `Motion at ${captured}`);
    const image = document.createElement("img");
    image.src = frameSrc(event.id, 0);
    image.alt = "";
    image.loading = "lazy";
    button.append(image);
    button.addEventListener("click", () => openLightbox(event, 0));
    eventsStrip.append(button);
  }
}

function openLightbox(event: MotionEvent, frame: number): void {
  lightboxEvent = event;
  lightboxFrame = Math.max(0, Math.min(frame, event.frames - 1));
  updateLightbox();
  if (!lightbox.open) {
    lightbox.showModal();
  }
}

function updateLightbox(): void {
  if (!lightboxEvent) {
    return;
  }
  const captured = eventTime.format(new Date(lightboxEvent.capturedAt));
  lightboxImage.src = frameSrc(lightboxEvent.id, lightboxFrame);
  lightboxImage.alt = `Motion capture at ${captured}`;
  lightboxCaption.textContent = `${captured} · ${lightboxFrame + 1} / ${lightboxEvent.frames} · score ${percent(lightboxEvent.score)}`;
  lightboxPrev.disabled = lightboxFrame <= 0;
  lightboxNext.disabled = lightboxFrame >= lightboxEvent.frames - 1;
}

function stepLightbox(delta: number): void {
  if (!lightboxEvent || !lightbox.open) {
    return;
  }
  openLightbox(lightboxEvent, lightboxFrame + delta);
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
    revision.textContent = current.revision ? `rev ${current.revision}` : "";
    revision.title = current.revision
      ? `Deployed source revision ${current.revision}`
      : "";
    renderMotion(current);

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
lightboxPrev.addEventListener("click", () => stepLightbox(-1));
lightboxNext.addEventListener("click", () => stepLightbox(1));
lightbox.addEventListener("click", (event) => {
  if (event.target === lightbox) {
    lightbox.close();
  }
});
document.addEventListener("keydown", (event) => {
  if (!lightbox.open) {
    return;
  }
  if (event.key === "ArrowLeft") {
    stepLightbox(-1);
  } else if (event.key === "ArrowRight") {
    stepLightbox(1);
  }
});
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) {
    void pollStatus();
  }
});

void pollStatus();
