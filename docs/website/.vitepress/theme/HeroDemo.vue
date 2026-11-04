<script setup>
import { Film } from '@lucide/vue';
</script>

<template>
  <section class="demo" aria-label="Parcela video demo">
    <div class="video-placeholder">
      <Film aria-hidden="true" />
      <p>Video demo coming soon</p>
    </div>
  </section>
</template>

<style scoped>
.demo {
  width: 100%;
  max-width: 1120px;
  margin: 0 auto;
  padding: 0 32px;
}
.video-placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  aspect-ratio: 16 / 9;
  border: 1px solid var(--border);
  border-radius: 16px;
  background: var(--secondary);
  color: var(--muted-foreground);
}
.video-placeholder svg {
  width: 28px;
  height: 28px;
  color: var(--subtle-foreground);
}
.video-placeholder p {
  margin: 0;
  font-size: 14px;
}
@media (max-width: 699px) {
  .demo { padding: 0 24px; }
}
</style>
