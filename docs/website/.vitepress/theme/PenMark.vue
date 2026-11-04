<script setup>
import { ref } from 'vue';
import { useEntrance } from './useHomeMotion.js';

// A hand-drawn underline that draws itself once the text scrolls into view.
const root = ref(null);
const { armed, playing: drawn } = useEntrance(root, { threshold: 0.5, rootMargin: '0px 0px -15% 0px' });
</script>

<template>
  <span ref="root" class="pen-mark" :class="{ armed, drawn }"
    ><slot /><svg viewBox="0 0 500 20" preserveAspectRatio="none" aria-hidden="true" focusable="false">
      <path pathLength="1" d="M3 13 C80 9 200 5 330 7 C410 8 465 10 497 15" />
    </svg></span
  >
</template>

<style scoped>
.pen-mark {
  position: relative;
  display: inline-block;
  white-space: nowrap;
}
svg {
  position: absolute;
  left: -2%;
  bottom: -0.1em;
  width: 104%;
  height: 0.22em;
  overflow: visible;
  color: var(--home-sky);
  pointer-events: none;
}
:root.dark svg {
  color: #21a8d2;
}
path {
  fill: none;
  stroke: currentColor;
  stroke-width: clamp(1.5px, 0.18vw, 2.4px);
  stroke-linecap: round;
  vector-effect: non-scaling-stroke;
  stroke-dasharray: 1;
  stroke-dashoffset: 0;
}
.pen-mark.armed:not(.drawn) path {
  stroke-dashoffset: 1;
}
.pen-mark.armed.drawn path {
  transition: stroke-dashoffset 0.9s cubic-bezier(0.3, 0, 0.2, 1) 0.15s;
}
</style>
