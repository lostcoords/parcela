<script setup>
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { withBase } from 'vitepress';
import { ArrowRight } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import CodeText from './CodeText.vue';
import { useEntrance } from './useHomeMotion.js';
import desktop from '@/media/app-desktop.jpg';
import phone from '@/media/app-phone.jpg';
import DeviceFrame from './DeviceFrame.vue';
import SectionHeading from './SectionHeading.vue';

const devices = [
  {
    key: 'local',
    title: 'Your machine',
    url: 'https://myapp.localhost',
    image: desktop,
    text: '`parcela up` runs the environment here.',
  },
  {
    key: 'phone',
    title: 'Your phone',
    url: 'https://myapp.local',
    image: phone,
    flag: 'parcela up --lan',
    text: 'A `.local` name on the same network.',
  },
  {
    key: 'tailnet',
    title: 'Another computer',
    url: 'https://mb.tail1234.ts.net',
    image: desktop,
    flag: 'parcela up --tailscale',
    text: 'Served with `tailscale serve` to your tailnet.',
  },
];

// Arrows run from this machine's screen to each other screen. They are measured, so they
// follow the layout at every width.
const net = ref(null);
const screens = {};
const figures = {};
const arrows = ref([]);
function measure() {
  if (!net.value) return;
  const box = net.value.getBoundingClientRect();
  const rect = (key) => {
    const r = key.getBoundingClientRect();
    return { left: r.left - box.left, top: r.top - box.top, width: r.width, height: r.height };
  };
  // They leave from under this machine's caption and stop above each other screen.
  const source = rect(figures.local);
  arrows.value = devices.filter((device) => device.flag).map((device, i) => {
    const target = rect(screens[device.key]);
    const x1 = source.left + source.width * (i === 0 ? 0.38 : 0.62);
    const y1 = source.top + source.height + 16;
    const x2 = target.left + target.width / 2;
    const y2 = target.top - 16;
    const k = (y2 - y1) * 0.55;
    // A slight lean on the control points keeps the stroke from looking ruled.
    const c1 = [x1 + (x2 - x1) * 0.08, y1 + k];
    const c2 = [x2 + (x1 - x2) * 0.04, y2 - k];
    const label = [(x1 + 3 * c1[0] + 3 * c2[0] + x2) / 8, (y1 + 3 * c1[1] + 3 * c2[1] + y2) / 8];
    return {
      key: device.key,
      // Narrow, only the flag fits on the arrow.
      flag: box.width < 600 ? device.flag.replace('parcela up ', '') : device.flag,
      line: `M${x1} ${y1} C${c1[0]} ${c1[1]} ${c2[0]} ${c2[1]} ${x2} ${y2}`,
      head: `M${x2 - 8} ${y2 - 11} L${x2} ${y2} L${x2 + 8.5} ${y2 - 10}`,
      label,
    };
  });
}

// In view, the arrows draw in and the other screens rise in.
const root = ref(null);
const { armed, playing } = useEntrance(root);
let resize;

onMounted(() => {
  resize = new ResizeObserver(measure);
  resize.observe(net.value);
  for (const element of [...Object.values(screens), figures.local]) resize.observe(element);
  measure();
});
onBeforeUnmount(() => resize?.disconnect());
</script>

<template>
  <section ref="root" class="home-section share" :class="{ armed, playing }" aria-labelledby="sharing">
    <div class="section-head center">
      <SectionHeading id="sharing">
        Share with other devices
      </SectionHeading>
      <p class="section-lead">
        Each run stays on this machine unless you pass <code>--lan</code> or <code>--tailscale</code>,
        or set the matching environment variable.
      </p>
      <div class="section-actions">
        <Button as-child><a :href="withBase('/docs/worktrees/#share-with-another-device')">Share an environment <ArrowRight aria-hidden="true" /></a></Button>
      </div>
    </div>
    <div ref="net" class="net">
      <svg class="arrows" aria-hidden="true">
        <g v-for="(arrow, i) in arrows" :key="arrow.key" :style="{ '--delay': `${i * 300}ms` }">
          <path class="stroke" pathLength="1" :d="arrow.line" />
          <path class="head" pathLength="1" :d="arrow.head" />
        </g>
      </svg>
      <figure v-for="(device, i) in devices" :key="device.key" :ref="(el) => { if (el) figures[device.key] = el; }" class="device" :class="device.key" :style="{ '--delay': `${i * 300}ms` }">
        <div :ref="(el) => { if (el) screens[device.key] = el; }" class="screen">
          <DeviceFrame :kind="device.key === 'phone' ? 'phone' : 'laptop'" :src="device.image" :url="device.url" />
        </div>
        <figcaption>
          <h3>{{ device.title }}</h3>
          <p><CodeText :text="device.text" /></p>
          <code v-if="device.flag" class="flag-text">{{ device.flag }}</code>
        </figcaption>
      </figure>
      <code
        v-for="(arrow, i) in arrows"
        :key="`flag-${arrow.key}`"
        class="flag"
        :style="{ left: `${arrow.label[0]}px`, top: `${arrow.label[1]}px`, '--delay': `${i * 300 + 500}ms` }"
        aria-hidden="true"
      >{{ arrow.flag }}</code>
    </div>
  </section>
</template>

<style scoped>
.net {
  position: relative;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  grid-template-areas: 'local local' 'phone tailnet';
  row-gap: 170px;
  column-gap: 32px;
  margin-top: 64px;
}
.arrows {
  position: absolute;
  inset: 0;
  z-index: 0;
  width: 100%;
  height: 100%;
  overflow: visible;
  color: #21a8d2;
  pointer-events: none;
}
.arrows path {
  fill: none;
  stroke: currentColor;
  stroke-width: 2.2px;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-dasharray: 1;
  stroke-dashoffset: 0;
}
.device {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  margin: 0;
  text-align: center;
}
.local {
  grid-area: local;
}
.phone {
  grid-area: phone;
}
.tailnet {
  grid-area: tailnet;
}
.screen {
  position: relative;
  width: min(400px, 100%);
}
.local .screen {
  width: min(520px, 100%);
}
.phone .screen {
  width: 140px;
}
h3 {
  margin: 22px 0 0;
  font-size: 19px;
  font-weight: 750;
  letter-spacing: -0.02em;
}
figcaption p {
  margin: 6px 0 0;
  color: var(--muted-foreground);
  font-size: 15px;
  line-height: 1.6;
}
/* The flag sits on its arrow. The caption keeps a copy for screen readers. */
.flag {
  position: absolute;
  z-index: 2;
  padding: 3px 10px;
  border-radius: 999px;
  background: var(--background);
  color: var(--foreground);
  font-size: 13px;
  white-space: nowrap;
  transform: translate(-50%, -50%);
}
.flag-text {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
  white-space: nowrap;
}

/* Arrows draw from this machine, then each other screen rises in. Elements hide instantly when
   armed and only transition once playing. */
.armed:not(.playing) .arrows path {
  stroke-dashoffset: 1;
}
.armed:not(.playing) .flag,
.armed:not(.playing) .device:not(.local) {
  opacity: 0;
}
.armed:not(.playing) .device:not(.local) {
  transform: translateY(12px);
}
.armed.playing .arrows .stroke {
  transition: stroke-dashoffset 0.8s cubic-bezier(0.3, 0, 0.2, 1) var(--delay);
}
.armed.playing .arrows .head {
  transition: stroke-dashoffset 0.25s ease calc(var(--delay) + 0.75s);
}
.armed.playing .flag {
  transition: opacity 0.3s ease var(--delay);
}
.armed.playing .device {
  transition: opacity 0.45s ease, transform 0.6s cubic-bezier(0.2, 0.8, 0.2, 1);
  transition-delay: calc(var(--delay) + 0.6s);
}

@media (max-width: 699px) {
  .net {
    row-gap: 130px;
    column-gap: 16px;
    margin-top: 48px;
  }
  .phone .screen {
    width: min(104px, 100%);
  }
  h3 {
    font-size: 16px;
  }
  figcaption p {
    font-size: 13px;
  }
  .flag {
    font-size: 11px;
    padding: 2px 6px;
  }
}
</style>
