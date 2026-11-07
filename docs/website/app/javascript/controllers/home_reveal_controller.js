import { Controller } from "@hotwired/stimulus"

// Fades and lifts each landing page block once it scrolls into view. Blocks are marked only
// here and only when motion is allowed, so without either they are simply shown.
const BLOCKS = [
  ".section-head > *",
  ".stack-label",
  ".marquee",
  ".compare .side",
  ".bento > .tile",
  ".share .device.local",
  ".capability",
  ".questions-intro",
  ".question-list > *",
  ".cta .mascot-reveal",
  ".cta .pitch > *",
].join(", ")

export default class extends Controller {
  connect() {
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return
    this.observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue
        entry.target.classList.add("revealed")
        this.observer.unobserve(entry.target)
      }
    }, { rootMargin: "0px 0px -8% 0px" })
    for (const element of this.element.querySelectorAll(BLOCKS)) {
      // Siblings that enter together arrive one after another.
      const index = [...element.parentElement.children].filter((child) => child.matches(BLOCKS)).indexOf(element)
      element.style.setProperty("--reveal-delay", `${Math.min(index, 5) * 80}ms`)
      element.classList.add("reveal")
      this.observer.observe(element)
    }
  }

  disconnect() {
    this.observer?.disconnect()
  }
}
