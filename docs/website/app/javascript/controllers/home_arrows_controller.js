import { Controller } from "@hotwired/stimulus"

// The sharing diagram's arrows run from under this machine's caption to above each other
// screen. They are measured, so they follow the layout at every width.
export default class extends Controller {
  static targets = ["svg", "stroke", "head", "flag", "figure", "screen"]

  connect() {
    this.resize = new ResizeObserver(() => this.measure())
    this.resize.observe(this.element)
    this.screenTargets.forEach((screen) => this.resize.observe(screen))
    this.measure()
  }

  disconnect() {
    this.resize.disconnect()
  }

  measure() {
    const box = this.element.getBoundingClientRect()
    const rect = (element) => {
      const r = element.getBoundingClientRect()
      return { left: r.left - box.left, top: r.top - box.top, width: r.width, height: r.height }
    }
    const source = rect(this.figureTargets[0])
    this.screenTargets.slice(1).forEach((screen, i) => {
      const target = rect(screen)
      const x1 = source.left + source.width * (i === 0 ? 0.38 : 0.62)
      const y1 = source.top + source.height + 16
      const x2 = target.left + target.width / 2
      const y2 = target.top - 16
      const k = (y2 - y1) * 0.55
      // A slight lean on the control points keeps the stroke from looking ruled.
      const c1 = [x1 + (x2 - x1) * 0.08, y1 + k]
      const c2 = [x2 + (x1 - x2) * 0.04, y2 - k]
      this.strokeTargets[i].setAttribute("d", `M${x1} ${y1} C${c1[0]} ${c1[1]} ${c2[0]} ${c2[1]} ${x2} ${y2}`)
      this.headTargets[i].setAttribute("d", `M${x2 - 8} ${y2 - 11} L${x2} ${y2} L${x2 + 8.5} ${y2 - 10}`)
      const flag = this.flagTargets[i]
      // Narrow, only the flag fits on the arrow.
      flag.textContent = box.width < 600 ? flag.dataset.short : flag.dataset.full
      flag.style.left = `${(x1 + 3 * c1[0] + 3 * c2[0] + x2) / 8}px`
      flag.style.top = `${(y1 + 3 * c1[1] + 3 * c2[1] + y2) / 8}px`
    })
  }
}
