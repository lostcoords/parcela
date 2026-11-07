import { Controller } from "@hotwired/stimulus"

// A one-time entrance: "armed" while motion is allowed, "playing" once the element scrolls into
// view. Without JavaScript or with reduced motion it is never armed, so everything is shown.
// By default it plays when the top passes 80% of the viewport: a share of the element's own
// height can exceed a short screen and never be reached.
export default class extends Controller {
  static values = { threshold: { type: Number, default: 0 }, margin: { type: String, default: "0px 0px -20% 0px" } }

  connect() {
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return
    this.element.classList.add("armed")
    this.observer = new IntersectionObserver(([entry]) => {
      if (!entry.isIntersecting) return
      this.element.classList.add("playing")
      this.observer.disconnect()
    }, { threshold: this.thresholdValue, rootMargin: this.marginValue })
    this.observer.observe(this.element)
  }

  disconnect() {
    this.observer?.disconnect()
  }
}
