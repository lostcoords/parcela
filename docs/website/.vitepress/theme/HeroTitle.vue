<script setup>
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useHomeMotion } from "./useHomeMotion.js";
import { agents, github } from "./agents.js";
import { useData, withBase } from "vitepress";
import { ArrowRight, ArrowUpRight, Tag } from "@lucide/vue";
import { Button } from "@/components/ui/button";
import InstallCommand from "./InstallCommand.vue";
import AgentInstall from "./AgentInstall.vue";

const { theme } = useData();
const moving = useHomeMotion();
const current = ref(0);
const width = ref(null);
const items = ref([]);
let timer;
let resize;
function measure() {
  width.value = items.value[current.value]?.offsetWidth ?? null;
}
watch(moving, (enabled) => {
  clearInterval(timer);
  if (enabled) timer = setInterval(async () => {
    current.value = (current.value + 1) % agents.length;
    await nextTick();
    measure();
  }, 2400);
});
onMounted(() => {
  resize = new ResizeObserver(measure);
  items.value.forEach((item) => resize.observe(item));
  measure();
});
onBeforeUnmount(() => {
  clearInterval(timer);
  resize?.disconnect();
});
</script>

<template>
  <div class="hero-intro">
    <a class="release-pill" :href="theme.release.link">
      <Tag aria-hidden="true" />
      Parcela {{ theme.release.tag }} is out <ArrowUpRight aria-hidden="true" />
    </a>
    <h1 class="heading">
      <span class="text"
        ><span class="line"
          >Isolated
          <span class="marked"
            >dev environments<svg
              class="pen-underline"
              viewBox="0 0 500 24"
              preserveAspectRatio="none"
              aria-hidden="true"
              focusable="false"
            >
              <path
                pathLength="1"
                d="M4 8 C126 5 326 8 495 6 C380 10 231 12 141 15 C212 16 291 18 350 21"
              /></svg></span
        ></span>
        <span class="line">for your <span class="sr">coding agents</span><span
          class="rotator"
          :style="width ? { width: `${width}px` } : null"
          aria-hidden="true"
        ><span
          v-for="(agent, i) in agents"
          :key="agent.name"
          ref="items"
          class="agent"
          :class="{ on: i === current, past: i === (current + agents.length - 1) % agents.length }"
          :style="agent.color ? { color: agent.color } : null"
        ><svg viewBox="0 0 24 24"><path :d="agent.path" fill="currentColor" /></svg><span
          class="name"
          :style="agent.gradient ? { backgroundImage: agent.gradient } : null"
          :class="{ blend: agent.gradient }"
        >{{ agent.name }}</span></span></span></span>
      </span>
    </h1>
    <nav class="hero-actions" aria-label="Get Parcela">
      <Button as-child size="lg" class="home-start">
        <a :href="withBase('/docs/getting-started/')"
          >Get started <ArrowRight aria-hidden="true"
        /></a>
      </Button>
      <Button as-child variant="ghost" size="lg">
        <a href="https://github.com/lostcoords/parcela">
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path :d="github" fill="currentColor" />
          </svg>
          GitHub
        </a>
      </Button>
    </nav>
    <InstallCommand class="install" />
    <p class="agent-install"><AgentInstall /></p>
  </div>
</template>

<style scoped>
.hero-intro {
  width: 100%;
  text-align: center;
}
.release-pill {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  border-radius: 999px;
  background:
    linear-gradient(
      135deg,
      rgb(255 255 255 / 0.75),
      rgb(255 255 255 / 0.25) 48%,
      rgb(147 217 247 / 0.17)
    ),
    var(--glass-fill);
  box-shadow:
    inset 0 1px 1px rgb(255 255 255 / 0.9),
    inset 0 0 0 1px rgb(255 255 255 / 0.45),
    0 2px 4px rgb(0 0 0 / 0.04),
    0 8px 24px -12px rgb(34 106 150 / 0.25);
  backdrop-filter: blur(16px) saturate(150%);
  -webkit-backdrop-filter: blur(16px) saturate(150%);
  margin: 0 0 24px;
  color: var(--foreground);
  font-size: 13px;
  font-weight: 600;
  text-decoration: none;
  transition: color 0.2s ease;
}
.release-pill svg {
  flex: none;
  width: 14px;
  height: 14px;
}
.release-pill:hover {
  color: var(--home-sky);
}
:root.dark .release-pill {
  background:
    linear-gradient(
      135deg,
      rgb(255 255 255 / 0.12),
      rgb(255 255 255 / 0.025) 50%,
      rgb(33 168 210 / 0.12)
    ),
    var(--glass-fill);
  box-shadow:
    inset 0 1px 0 rgb(255 255 255 / 0.18),
    inset 0 0 0 1px rgb(255 255 255 / 0.07),
    0 4px 16px rgb(0 0 0 / 0.16);
}
:root.dark .release-pill:hover {
  color: #65c6e3;
}
.heading {
  margin: 0;
  color: var(--foreground);
  font-size: clamp(36px, 5.5vw, 68px);
  font-weight: 800;
  letter-spacing: -0.045em;
  line-height: 1.12;
}
.hero-actions {
  display: flex;
  justify-content: center;
  flex-wrap: wrap;
  gap: 12px;
  margin-top: 30px;
}
.hero-actions a:not(.home-start) {
  border: 0;
  padding-inline: 14px;
  font-size: 14px;
}
.install {
  margin: 26px auto 0;
  max-width: 480px;
}
.agent-install {
  margin: 8px 0 0;
}
.agent-install :deep(.link) {
  color: var(--muted-foreground);
  font-size: 13px;
  font-weight: 500;
}
@media (max-width: 639px) {
  .release-pill {
    margin-bottom: 20px;
    font-size: 14px;
  }
  .heading {
    font-size: clamp(30px, 7.6vw, 46px);
    letter-spacing: -0.04em;
  }
  .wide-break {
    display: none;
  }
  .install :deep(code) {
    padding: 0 42px 0 12px;
    font-size: clamp(9px, 2.5vw, 12px);
    letter-spacing: -0.03em;
  }
}
.line {
  display: block;
}

.marked {
  position: relative;
  display: inline-block;
  white-space: nowrap;
}
.pen-underline {
  position: absolute;
  left: 0;
  bottom: -0.06em;
  width: 100%;
  height: 0.24em;
  overflow: visible;
  color: var(--home-sky);
  pointer-events: none;
}
.pen-underline path {
  fill: none;
  stroke: currentColor;
  stroke-width: clamp(1.5px, 0.18vw, 2.6px);
  stroke-linecap: round;
  stroke-linejoin: round;
  vector-effect: non-scaling-stroke;
  stroke-dasharray: 1;
  stroke-dashoffset: 0;
}
:root.dark .pen-underline {
  color: #21a8d2;
}
@media (prefers-reduced-transparency: reduce) {
  .release-pill,
  :root.dark .release-pill {
    background: var(--secondary);
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }
}
.sr {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
  white-space: nowrap;
}

.rotator {
  position: relative;
  display: inline-block;
  height: 1.12em;
  overflow: hidden;
  vertical-align: bottom;
  text-align: left;
  transition: width 0.45s cubic-bezier(0.45, 0, 0.25, 1);
}

.agent {
  position: absolute;
  left: 0;
  top: 0;
  display: inline-flex;
  align-items: center;
  gap: 0.22em;
  color: var(--vp-c-text-1);
  white-space: nowrap;
  opacity: 0;
  transform: translateY(100%);
  transition: transform 0.45s cubic-bezier(0.45, 0, 0.25, 1), opacity 0.45s ease;
}

/* The first item holds the box open before the script measures. */
.agent:first-child {
  position: relative;
}

.agent.on {
  opacity: 1;
  transform: none;
}

.agent.past {
  transform: translateY(-100%);
}

.blend {
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}

.agent svg {
  flex: none;
  width: 0.78em;
  height: 0.78em;
}

@media (prefers-reduced-motion: no-preference) {
  .pen-underline path {
    animation: underline-draw 1.1s ease-out 0.3s both;
  }
}
@keyframes underline-draw {
  from { stroke-dashoffset: 1; }
  to { stroke-dashoffset: 0; }
}
</style>
