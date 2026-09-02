import "./style.css";

export type Phase = "starting" | "live" | "offline";

export interface MotionStatus {
  score: number;
  threshold: number;
  detecting: boolean;
}

export interface MotionEvent {
  id: string;
  capturedAt: number;
  frames: number;
  score: number;
}

export interface StreamStatus {
  phase: Phase;
  message: string | null;
  startedAt: number | null;
  restarts: number;
  revision: string;
  motion: MotionStatus;
  events: MotionEvent[];
}

export interface StatusElements {
  status: HTMLDivElement;
  statusLabel: HTMLSpanElement;
  message: HTMLParagraphElement;
  retry: HTMLButtonElement;
  connection: HTMLSpanElement;
  restarts: HTMLSpanElement;
  revision: HTMLSpanElement;
  motionScore: HTMLSpanElement;
  eventsEmpty: HTMLParagraphElement;
  eventsStrip: HTMLDivElement;
  lightbox: HTMLDialogElement;
  lightboxImage: HTMLImageElement;
  lightboxCaption: HTMLParagraphElement;
  lightboxPrev: HTMLButtonElement;
  lightboxNext: HTMLButtonElement;
  placeholder?: HTMLDivElement;
}

export function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) {
    throw new Error(`Missing required element #${id}`);
  }
  return found as T;
}

export function setPhase(
  elements: Pick<StatusElements, "status" | "statusLabel" | "message" | "retry" | "placeholder">,
  phase: Phase,
  detail?: string,
): void {
  elements.status.dataset.phase = phase;
  elements.statusLabel.textContent =
    phase === "live" ? "Live" : phase === "offline" ? "Offline" : "Connecting";
  elements.message.textContent =
    detail ??
    (phase === "offline"
      ? "The camera is currently unavailable."
      : "Waiting for the camera stream…");
  if (elements.placeholder) {
    elements.placeholder.hidden = phase === "live";
  }
  elements.retry.hidden = phase !== "offline";
}

export function percent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}

export function frameSrc(id: string, frame: number): string {
  return `/events/${id}/${String(frame).padStart(2, "0")}.jpg`;
}

const eventTime = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  second: "2-digit",
});

export function createMotionController(elements: StatusElements) {
  let events: MotionEvent[] = [];
  let renderedEventKey = "";
  let lightboxEvent: MotionEvent | null = null;
  let lightboxFrame = 0;

  function updateLightbox(): void {
    if (!lightboxEvent) {
      return;
    }
    const captured = eventTime.format(new Date(lightboxEvent.capturedAt));
    elements.lightboxImage.src = frameSrc(lightboxEvent.id, lightboxFrame);
    elements.lightboxImage.alt = `Motion capture at ${captured}`;
    elements.lightboxCaption.textContent = `${captured} · ${lightboxFrame + 1} / ${lightboxEvent.frames} · score ${percent(lightboxEvent.score)}`;
    elements.lightboxPrev.disabled = lightboxFrame <= 0;
    elements.lightboxNext.disabled = lightboxFrame >= lightboxEvent.frames - 1;
  }

  function openLightbox(event: MotionEvent, frame: number): void {
    lightboxEvent = event;
    lightboxFrame = Math.max(0, Math.min(frame, event.frames - 1));
    updateLightbox();
    if (!elements.lightbox.open) {
      elements.lightbox.showModal();
    }
  }

  function stepLightbox(delta: number): void {
    if (!lightboxEvent || !elements.lightbox.open) {
      return;
    }
    openLightbox(lightboxEvent, lightboxFrame + delta);
  }

  function renderMotion(current: StreamStatus): void {
    if (current.motion.detecting) {
      elements.motionScore.textContent = `${percent(current.motion.score)} / ${percent(current.motion.threshold)}`;
      elements.motionScore.title = "Latest motion score versus the capture threshold";
    } else {
      elements.motionScore.textContent = "Waiting";
      elements.motionScore.title = "Motion analysis starts a few seconds after the stream is live";
    }

    events = current.events;
    const key = events.map((event) => event.id).join(",");
    if (key === renderedEventKey) {
      return;
    }
    renderedEventKey = key;
    elements.eventsEmpty.hidden = events.length > 0;
    elements.eventsStrip.hidden = events.length === 0;
    elements.eventsStrip.replaceChildren();
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
      elements.eventsStrip.append(button);
    }
  }

  function updateStatusMeta(current: StreamStatus, liveConnectionText: string): void {
    elements.connection.textContent =
      current.phase === "live" ? liveConnectionText : "Waiting for the Pi";
    elements.restarts.textContent =
      current.restarts > 0
        ? `${current.restarts} pipeline restart${current.restarts === 1 ? "" : "s"}`
        : "";
    elements.revision.textContent = current.revision ? `rev ${current.revision}` : "";
    elements.revision.title = current.revision
      ? `Deployed source revision ${current.revision}`
      : "";
  }

  elements.lightboxPrev.addEventListener("click", () => stepLightbox(-1));
  elements.lightboxNext.addEventListener("click", () => stepLightbox(1));
  elements.lightbox.addEventListener("click", (event) => {
    if (event.target === elements.lightbox) {
      elements.lightbox.close();
    }
  });
  document.addEventListener("keydown", (event) => {
    if (!elements.lightbox.open) {
      return;
    }
    if (event.key === "ArrowLeft") {
      stepLightbox(-1);
    } else if (event.key === "ArrowRight") {
      stepLightbox(1);
    }
  });

  return { renderMotion, updateStatusMeta, openLightbox };
}

export function createStatusPoller(
  elements: StatusElements,
  options: {
    liveConnectionText: string;
    onLive?: (current: StreamStatus) => void | Promise<void>;
    onOffline?: () => void;
  },
) {
  const motion = createMotionController(elements);
  let pollTimer: number | undefined;

  async function pollStatus(): Promise<void> {
    try {
      const response = await fetch("/api/status", { cache: "no-store" });
      if (!response.ok) {
        throw new Error(`status returned ${response.status}`);
      }
      const current = (await response.json()) as StreamStatus;
      setPhase(elements, current.phase, current.message ?? undefined);
      motion.updateStatusMeta(current, options.liveConnectionText);
      motion.renderMotion(current);

      if (current.phase === "live") {
        await options.onLive?.(current);
      }
    } catch {
      setPhase(elements, "offline", "The Pi monitor could not be reached.");
      elements.connection.textContent = "Check Tailscale and the Pi";
      options.onOffline?.();
    } finally {
      window.clearTimeout(pollTimer);
      pollTimer = window.setTimeout(() => {
        void pollStatus();
      }, 3_000);
    }
  }

  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) {
      void pollStatus();
    }
  });

  return {
    pollStatus,
    renderMotion: motion.renderMotion,
    updateStatusMeta: motion.updateStatusMeta,
  };
}
