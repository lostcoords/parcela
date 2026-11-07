import { Controller } from "@hotwired/stimulus"

// The hero's agent name: every 2.4 s the next one rolls up into place, and the box eases to
// its width. Without motion the first name stays.
export default class extends Controller {
  static targets = ["item"]

  connect() {
    this.current = 0
    this.preference = matchMedia("(prefers-reduced-motion: reduce)")
    this.update = () => this.schedule()
    this.preference.addEventListener("change", this.update)
    this.resize = new ResizeObserver(() => this.measure())
    this.itemTargets.forEach((item) => this.resize.observe(item))
    this.measure()
    this.schedule()
  }

  disconnect() {
    clearInterval(this.timer)
    this.resize.disconnect()
    this.preference.removeEventListener("change", this.update)
  }

  schedule() {
    clearInterval(this.timer)
    if (!this.preference.matches) this.timer = setInterval(() => this.advance(), 2400)
  }

  advance() {
    const count = this.itemTargets.length
    this.current = (this.current + 1) % count
    this.itemTargets.forEach((item, index) => {
      item.classList.toggle("on", index === this.current)
      item.classList.toggle("past", index === (this.current + count - 1) % count)
    })
    this.measure()
  }

  measure() {
    const width = this.itemTargets[this.current]?.offsetWidth
    if (width) this.element.style.width = `${width}px`
  }
}
