// Fades and lifts each homepage block once it scrolls into view. Blocks are marked only from
// JavaScript and only when motion is allowed, so without either they are simply shown.
const blocks = [
  '.VPHome .section-head > *',
  '.VPHome .stack-label',
  '.VPHome .marquee',
  '.VPHome .compare .side',
  '.VPHome .bento > .tile',
  '.VPHome .dashboard-stage',
  '.VPHome .share .device.local',
  '.VPHome .capability',
  '.VPHome .questions-intro',
  '.VPHome .question-list > *',
  '.VPHome .cta .mascot-reveal',
  '.VPHome .cta .pitch > *',
  '.Layout:has(.VPHome) .footer .inner > *',
].join(', ');

export function reveal() {
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return () => {};
  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      entry.target.classList.add('revealed');
      observer.unobserve(entry.target);
    }
  }, { rootMargin: '0px 0px -8% 0px' });
  for (const element of document.querySelectorAll(blocks)) {
    // Siblings that enter together arrive one after another.
    const index = [...element.parentElement.children].filter((child) => child.matches(blocks)).indexOf(element);
    element.style.setProperty('--reveal-delay', `${Math.min(index, 5) * 80}ms`);
    element.classList.add('reveal');
    observer.observe(element);
  }
  return () => observer.disconnect();
}
