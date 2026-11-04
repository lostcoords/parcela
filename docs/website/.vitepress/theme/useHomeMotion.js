import { onBeforeUnmount, onMounted, ref } from 'vue';

export function useHomeMotion() {
  const moving = ref(false);
  let preference;
  const update = () => { moving.value = !preference.matches; };
  onMounted(() => {
    preference = window.matchMedia('(prefers-reduced-motion: reduce)');
    update();
    preference.addEventListener('change', update);
  });
  onBeforeUnmount(() => preference?.removeEventListener('change', update));
  return moving;
}

// A one-time entrance that plays once `target` scrolls into view. It is armed only while
// motion is allowed, so without JavaScript or with reduced motion everything is shown.
// By default it plays when the target's top passes 65% of the viewport: a share of the
// target's own height can exceed a short screen and never be reached.
export function useEntrance(target, options = { rootMargin: '0px 0px -35% 0px' }) {
  const armed = useHomeMotion();
  const playing = ref(false);
  let observer;
  onMounted(() => {
    observer = new IntersectionObserver(([entry]) => {
      if (!entry.isIntersecting) return;
      playing.value = true;
      observer.disconnect();
    }, options);
    observer.observe(target.value);
  });
  onBeforeUnmount(() => observer?.disconnect());
  return { armed, playing };
}
