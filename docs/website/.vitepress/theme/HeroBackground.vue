<script setup>
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { useHomeMotion } from './useHomeMotion.js';

const moving = useHomeMotion();

const field = ref(null);
const mesh = ref(null);
let context;
let hero;
let observer;
let frame = 0;
let width = 0;
let height = 0;
let ratio = 1;
let active = false;
let targetX = -1000;
let targetY = -1000;
let pointerX = targetX;
let pointerY = targetY;

const spacing = 28;
const bulgeRadius = 280;
const revealRadius = 500;

function draw() {
  frame = 0;
  if (!active || !context) return;

  pointerX += (targetX - pointerX) * 0.32;
  pointerY += (targetY - pointerY) * 0.32;
  field.value.style.setProperty('--pointer-x', `${pointerX}px`);
  field.value.style.setProperty('--pointer-y', `${pointerY}px`);

  context.clearRect(0, 0, width, height);
  context.save();
  context.beginPath();
  context.arc(pointerX, pointerY, revealRadius, 0, Math.PI * 2);
  context.clip();

  const origin = spacing / 2;
  const firstX = Math.floor((pointerX - revealRadius - origin) / spacing) * spacing + origin;
  const lastX = Math.ceil((pointerX + revealRadius - origin) / spacing) * spacing + origin;
  const firstY = Math.floor((pointerY - revealRadius - origin) / spacing) * spacing + origin;
  const lastY = Math.ceil((pointerY + revealRadius - origin) / spacing) * spacing + origin;

  function point(x, y) {
    const dx = x - pointerX;
    const dy = y - pointerY;
    const distance = Math.hypot(dx, dy);
    const scaled = distance / bulgeRadius;
    const lift = Math.exp(-Math.pow(scaled / 0.55, 2));
    const push = 64 * scaled * lift;
    const direction = distance ? push / distance : 0;
    return {
      x: x + dx * direction,
      y: y + dy * direction - lift * 12,
      height: lift,
    };
  }

  context.fillStyle = getComputedStyle(field.value).color;
  for (let x = firstX; x <= lastX; x += spacing) {
    for (let y = firstY; y <= lastY; y += spacing) {
      const dot = point(x, y);
      context.globalAlpha = 0.64 + dot.height * 0.36;
      context.beginPath();
      context.arc(dot.x, dot.y, 1 + dot.height * 1.2, 0, Math.PI * 2);
      context.fill();
    }
  }
  context.restore();

  if (Math.abs(targetX - pointerX) > 0.5 || Math.abs(targetY - pointerY) > 0.5) {
    frame = requestAnimationFrame(draw);
  }
}

function scheduleDraw() {
  if (!frame) frame = requestAnimationFrame(draw);
}

function move(event) {
  if (!moving.value) return;
  if (event.pointerType !== 'mouse') return;
  const bounds = field.value.getBoundingClientRect();
  const x = event.clientX - bounds.left;
  const y = event.clientY - bounds.top;
  if (!active) {
    pointerX = x;
    pointerY = y;
  }
  targetX = x;
  targetY = y;
  active = true;
  field.value?.classList.add('active');
  scheduleDraw();
}

function leave() {
  active = false;
  field.value?.classList.remove('active');
}

function resize() {
  const bounds = hero.getBoundingClientRect();
  const home = field.value.parentElement.getBoundingClientRect();
  width = bounds.width;
  height = bounds.height;
  field.value.style.top = `${bounds.top - home.top}px`;
  field.value.style.height = `${height}px`;
  ratio = Math.min(window.devicePixelRatio || 1, 2);
  mesh.value.width = Math.round(width * ratio);
  mesh.value.height = Math.round(height * ratio);
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  scheduleDraw();
}

onMounted(() => {
  hero = field.value.parentElement.querySelector('.VPHero');
  context = mesh.value.getContext('2d');
  observer = new ResizeObserver(resize);
  observer.observe(hero);
  window.addEventListener('resize', resize, { passive: true });
  resize();
  if (!window.matchMedia('(hover: hover) and (pointer: fine)').matches) return;
  hero.addEventListener('pointermove', move, { passive: true });
  hero.addEventListener('pointerleave', leave);
});

onBeforeUnmount(() => {
  cancelAnimationFrame(frame);
  observer?.disconnect();
  hero?.removeEventListener('pointermove', move);
  hero?.removeEventListener('pointerleave', leave);
  window.removeEventListener('resize', resize);
});
</script>

<template>
  <div ref="field" class="background" :style="{ '--reveal-radius': `${revealRadius}px` }" aria-hidden="true">
    <canvas ref="mesh" class="mesh" />
  </div>
</template>

<style scoped>
.background {
  position: absolute;
  z-index: -1;
  top: 0;
  right: 0;
  left: 0;
  height: 0;
  overflow: hidden;
  pointer-events: none;
  color: var(--hero-dot);
  --pointer-x: -1000px;
  --pointer-y: -1000px;
  -webkit-mask-image: linear-gradient(#000 calc(100% - 100px), transparent);
  mask-image: linear-gradient(#000 calc(100% - 100px), transparent);
}

.background::before {
  position: absolute;
  inset: 0;
  background-image: radial-gradient(circle, var(--hero-dot) 1px, transparent 1.25px);
  background-size: 28px 28px;
  content: '';
  opacity: 0.16;
}

.mesh {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  opacity: 0;
  transition: opacity 180ms ease;
  -webkit-mask-image: radial-gradient(
    circle var(--reveal-radius) at var(--pointer-x) var(--pointer-y),
    #000 0%,
    rgb(0 0 0 / 0.72) 48%,
    transparent 100%
  );
  mask-image: radial-gradient(
    circle var(--reveal-radius) at var(--pointer-x) var(--pointer-y),
    #000 0%,
    rgb(0 0 0 / 0.72) 48%,
    transparent 100%
  );
}

.background.active .mesh {
  opacity: 0.92;
}

@media (prefers-reduced-transparency: reduce) {
  .background {
    display: none;
  }
}

@media (max-width: 767px) {
  .background::before {
    opacity: 0.08;
  }

  .mesh {
    display: none;
  }
}
</style>
