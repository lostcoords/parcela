<script setup>
import { ref, watch } from 'vue';
import { withBase } from 'vitepress';
import { ArrowRight } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import dashboardVideo from '@/media/dashboard.mp4';
import dashboardPoster from '@/media/dashboard-poster.jpg';
import { useHomeMotion } from './useHomeMotion.js';
import SectionHeading from './SectionHeading.vue';

// A recording of the real dashboard. It plays on its own only when motion is allowed.
const moving = useHomeMotion();
const video = ref(null);
watch([moving, video], ([enabled, element]) => {
  if (!element) return;
  if (enabled) element.play().catch(() => {});
  else element.pause();
});
</script>

<template>
  <section class="home-section dashboard" aria-labelledby="dashboard">
    <div class="section-head center">
      <SectionHeading id="dashboard">
        See every environment in the dashboard
      </SectionHeading>
      <p class="section-lead">
        <code>parcela ui</code> prints the dashboard address. The dashboard lists environments by
        project with their URLs and process states, and opens any of them next to the list. To switch
        from inside your app, add the optional side tab script.
      </p>
      <div class="section-actions">
        <Button as-child><a :href="withBase('/docs/cli/#ui')">Open the dashboard <ArrowRight aria-hidden="true" /></a></Button>
      </div>
    </div>
    <figure class="dashboard-stage">
      <video
        ref="video"
        :src="dashboardVideo"
        :poster="dashboardPoster"
        :controls="!moving"
        muted
        loop
        playsinline
        preload="metadata"
        width="1600"
        height="1000"
        aria-label="Recording of the Parcela dashboard: searching environments, then opening sale-prices and stock-filter beside the switcher"
      ></video>
    </figure>
  </section>
</template>

<style scoped>
.dashboard-stage {
  margin: 56px 0 0;
}
.dashboard-stage video {
  display: block;
  width: 100%;
  height: auto;
  aspect-ratio: 16 / 10;
  border-radius: 16px;
  background: var(--secondary);
}
:root:not(.dark) .dashboard-stage video {
  box-shadow: 0 30px 60px -36px rgb(40 40 30 / 0.4);
}
@media (max-width: 699px) {
  .dashboard-stage {
    margin-top: 40px;
  }
  .dashboard-stage video {
    border-radius: 12px;
  }
}
</style>
