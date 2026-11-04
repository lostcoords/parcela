import { h, onMounted, watch } from 'vue';
import { useRoute } from 'vitepress';
import DefaultTheme from 'vitepress/theme';
import HeroBackground from './HeroBackground.vue';
import HeroDemo from './HeroDemo.vue';
import HeroTitle from './HeroTitle.vue';
import HomeCta from './HomeCta.vue';
import HomeDashboard from './HomeDashboard.vue';
import HomeSharing from './HomeSharing.vue';
import HomeStack from './HomeStack.vue';
import HomeCompare from './HomeCompare.vue';
import HomeSteps from './HomeSteps.vue';
import HomeFeatures from './HomeFeatures.vue';
import SiteFooter from './SiteFooter.vue';
import NavSections from './NavSections.js';
import { reveal } from './reveal.js';
import '../../src/parcela.css';
import '../../src/theme.css';
import '../../src/home.css';

export default {
  extends: DefaultTheme,
  setup() {
    if (import.meta.env.SSR) return;
    const route = useRoute();
    let entrance;
    let stopReveal = () => {};
    const revealHome = () => {
      stopReveal();
      stopReveal = document.querySelector('.VPHome') ? reveal() : () => {};
    };
    onMounted(revealHome);
    watch(() => route.path, () => {
      entrance?.cancel();
      revealHome();
      if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
      const article = document.querySelector('.VPDoc .content-container');
      entrance = article?.animate([
        { opacity: 0 },
        { opacity: 1 },
      ], { duration: 280, easing: 'ease-out' });
    }, { flush: 'post' });
  },
  Layout: () => h(DefaultTheme.Layout, null, {
    'home-hero-before': () => h(HeroBackground),
    'nav-bar-content-before': () => h(NavSections),
    'home-hero-info': () => h(HeroTitle),
    'home-hero-after': () => h(HeroDemo),
    'home-features-after': () => [h(HomeStack), h(HomeCompare), h(HomeSteps), h(HomeDashboard), h(HomeSharing), h(HomeFeatures), h(HomeCta)],
    'layout-bottom': () => h(SiteFooter),
  }),
};
