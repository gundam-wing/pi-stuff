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
const latency = element<HTMLSpanElement>("latency");

let peer: RTCPeerConnection | null = null;
let retryTimer: number | undefined;
let playerConnected = false;
let statsTimer: number | undefined;

function scheduleReconnect(delay = 3_000): void {
  window.clearTimeout(retryTimer);
  retryTimer = window.setTimeout(() => {
    void connectPlayer();
  }, delay);
}

function disconnectPlayer(): void {
  window.clearTimeout(statsTimer);
  peer?.close();
  peer = null;
  video.srcObject = null;
  playerConnected = false;
  latency.textContent = "";
}

async function waitForIceGathering(pc: RTCPeerConnection): Promise<void> {
  if (pc.iceGatheringState === "complete") {
    return;
  }

  await new Promise<void>((resolve) => {
    const checkState = (): void => {
      if (pc.iceGatheringState === "complete") {
        pc.removeEventListener("icegatheringstatechange", checkState);
        resolve();
      }
    };
    pc.addEventListener("icegatheringstatechange", checkState);
  });
}

async function updateLatency(pc: RTCPeerConnection): Promise<void> {
  const reports = await pc.getStats();
  for (const report of reports.values()) {
    if (report.type === "inbound-rtp" && report.kind === "video") {
      const jitterBuffer = report.jitterBufferDelay as number | undefined;
      const emitted = report.jitterBufferEmittedCount as number | undefined;
      if (jitterBuffer && emitted) {
        const averageMs = (jitterBuffer / emitted) * 1000;
        latency.textContent = `~${averageMs.toFixed(0)} ms buffer`;
        return;
      }
    }
  }
  latency.textContent = "Connected";
}

async function connectPlayer(): Promise<void> {
  window.clearTimeout(retryTimer);
  disconnectPlayer();
  setPhase(elements, "starting", "Negotiating WebRTC session…");

  const pc = new RTCPeerConnection({
    iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
  });
  peer = pc;

  pc.addTransceiver("video", { direction: "recvonly" });
  pc.ontrack = (event) => {
    const [stream] = event.streams;
    if (!stream) {
      return;
    }
    video.srcObject = stream;
    playerConnected = true;
    void video.play().catch(() => {
      elements.connection.textContent = "Tap play to start video";
    });
  };
  pc.onconnectionstatechange = () => {
    if (pc.connectionState === "failed" || pc.connectionState === "closed") {
      setPhase(elements, "offline", "WebRTC disconnected. Reconnecting…");
      scheduleReconnect();
    }
  };

  const offer = await pc.createOffer();
  await pc.setLocalDescription(offer);
  await waitForIceGathering(pc);

  const response = await fetch("/api/webrtc/offer", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      type: pc.localDescription?.type,
      sdp: pc.localDescription?.sdp,
    }),
  });

  if (!response.ok) {
    const payload = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(payload?.error ?? `WebRTC offer failed (${response.status})`);
  }

  const answer = (await response.json()) as RTCSessionDescriptionInit;
  await pc.setRemoteDescription(answer);
  statsTimer = window.setInterval(() => {
    void updateLatency(pc);
  }, 2_000);
}

const { pollStatus } = createStatusPoller(elements, {
  liveConnectionText: "WebRTC over your Tailscale network",
  onLive: async () => {
    if (!playerConnected && !peer) {
      try {
        await connectPlayer();
      } catch (error) {
        const detail = error instanceof Error ? error.message : "WebRTC connection failed.";
        setPhase(elements, "offline", `${detail} Reconnecting…`);
        scheduleReconnect();
      }
    }
  },
  onOffline: () => {
    disconnectPlayer();
  },
});

video.addEventListener("playing", () => setPhase(elements, "live"));
elements.retry.addEventListener("click", () => {
  void connectPlayer();
  void pollStatus();
});

void pollStatus();
