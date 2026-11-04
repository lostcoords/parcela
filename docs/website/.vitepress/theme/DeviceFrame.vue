<script setup>
import { computed } from 'vue';
import { Lock } from '@lucide/vue';

// A laptop or phone around a page capture, in their real shapes: a MacBook's 16:10 and an
// iPhone's 19.5:9. The browser's address bar shows the secure local name the page is served at.
const props = defineProps({
  kind: { type: String, default: 'laptop' },
  src: { type: String, required: true },
  url: { type: String, required: true },
  alt: { type: String, default: '' },
});
const host = computed(() => props.url.replace(/^https:\/\//, ''));
</script>

<template>
  <div class="device-frame" :class="kind">
    <div class="body">
      <div class="glass">
        <div class="bar" :aria-label="url">
          <span class="field"><Lock aria-hidden="true" /><span class="host">{{ host }}</span></span>
        </div>
        <div class="page"><img :src="src" :alt="alt" decoding="async" /></div>
      </div>
    </div>
    <div v-if="kind === 'laptop'" class="base"></div>
  </div>
</template>

<style scoped>
.device-frame {
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 100%;
}
.body {
  box-sizing: border-box;
  width: 100%;
  border: 1px solid var(--border);
  background: var(--vp-c-bg);
  box-shadow: var(--surface-shadow);
}
.glass {
  position: relative;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: #fff;
  container-type: inline-size;
}
.page {
  position: relative;
  flex: 1;
}
img {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  object-position: top;
}

/* The address field, always light like the captured page. */
.bar {
  display: flex;
  flex: none;
  align-items: center;
  justify-content: center;
  background: #f3f3f4;
}
.field {
  display: flex;
  align-items: center;
  justify-content: center;
  min-width: 0;
  border-radius: 999px;
  background: #fff;
  color: #2b2d31;
  font-family: var(--vp-font-family-mono);
  white-space: nowrap;
}
.field svg {
  flex: none;
  color: #2f8a52;
}
.host {
  overflow: hidden;
  text-overflow: ellipsis;
}

.laptop .body {
  padding: 2.2%;
  border-radius: 14px 14px 4px 4px;
}
.laptop .glass {
  aspect-ratio: 16 / 10;
  border-radius: 5px;
}
.laptop .bar {
  height: 7cqw;
  border-bottom: 1px solid #e6e6e8;
}
.laptop .field {
  gap: 1cqw;
  width: 46%;
  height: 4.4cqw;
  font-size: 2.2cqw;
}
.laptop .field svg {
  width: 2.2cqw;
  height: 2.2cqw;
}
.base {
  box-sizing: border-box;
  width: 114%;
  height: 10px;
  border: 1px solid var(--border);
  border-radius: 2px 2px 12px 12px;
  background: var(--secondary);
}

/* Safari on iPhone keeps the address bar at the bottom. */
.phone .body {
  padding: 5px;
  border-radius: 26px;
}
.phone .glass {
  aspect-ratio: 9 / 19.5;
  border-radius: 21px;
}
.phone .bar {
  order: 1;
  height: 17cqw;
  padding: 0 5cqw 3cqw;
}
.phone .field {
  gap: 2.5cqw;
  width: 100%;
  height: 11cqw;
  box-shadow: 0 1px 3px rgb(0 0 0 / 0.08);
  font-size: 6.4cqw;
}
.phone .field svg {
  width: 6cqw;
  height: 6cqw;
}
/* The Dynamic Island. */
.phone .glass::before {
  content: '';
  position: absolute;
  z-index: 1;
  top: 3.5%;
  left: 50%;
  width: 32%;
  height: 4.5%;
  border-radius: 999px;
  background: #111;
  transform: translateX(-50%);
}
</style>
