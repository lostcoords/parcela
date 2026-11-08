import { Controller } from "@hotwired/stimulus"

// The dot field behind the hero. Under a fine pointer, with motion allowed, the dots near the
// pointer bulge away from it on a canvas.
const SPACING = 28
const BULGE = 280
const REVEAL = 500

export default class extends Controller {
  static targets = ["field", "mesh"]

  connect() {
    this.canvas = this.meshTarget.getContext("2d")
    this.aim = { x: -1000, y: -1000 }
    this.pointer = { ...this.aim }
    this.active = false
    this.frame = 0
    this.observer = new ResizeObserver(() => this.resize())
    this.observer.observe(this.element)
    this.resize()
    if (!matchMedia("(hover: hover) and (pointer: fine)").matches) return
    this.onMove = (event) => this.move(event)
    this.onLeave = () => this.leave()
    this.element.addEventListener("pointermove", this.onMove, { passive: true })
    this.element.addEventListener("pointerleave", this.onLeave)
  }

  disconnect() {
    cancelAnimationFrame(this.frame)
    this.observer.disconnect()
    this.element.removeEventListener("pointermove", this.onMove)
    this.element.removeEventListener("pointerleave", this.onLeave)
  }

  move(event) {
    if (event.pointerType !== "mouse" || matchMedia("(prefers-reduced-motion: reduce)").matches) return
    const bounds = this.fieldTarget.getBoundingClientRect()
    const x = event.clientX - bounds.left
    const y = event.clientY - bounds.top
    if (!this.active) this.pointer = { x, y }
    this.aim = { x, y }
    this.active = true
    this.fieldTarget.classList.add("active")
    this.schedule()
  }

  leave() {
    this.active = false
    this.fieldTarget.classList.remove("active")
  }

  resize() {
    const bounds = this.fieldTarget.getBoundingClientRect()
    this.width = bounds.width
    this.height = bounds.height
    const ratio = Math.min(window.devicePixelRatio || 1, 2)
    this.meshTarget.width = Math.round(this.width * ratio)
    this.meshTarget.height = Math.round(this.height * ratio)
    this.canvas.setTransform(ratio, 0, 0, ratio, 0, 0)
    this.schedule()
  }

  schedule() {
    if (!this.frame) this.frame = requestAnimationFrame(() => this.draw())
  }

  draw() {
    this.frame = 0
    if (!this.active) return
    const { canvas: context, pointer, aim: target } = this
    pointer.x += (target.x - pointer.x) * 0.32
    pointer.y += (target.y - pointer.y) * 0.32
    this.fieldTarget.style.setProperty("--pointer-x", `${pointer.x}px`)
    this.fieldTarget.style.setProperty("--pointer-y", `${pointer.y}px`)

    context.clearRect(0, 0, this.width, this.height)
    context.save()
    context.beginPath()
    context.arc(pointer.x, pointer.y, REVEAL, 0, Math.PI * 2)
    context.clip()
    const origin = SPACING / 2
    const span = (value) => [
      Math.floor((value - REVEAL - origin) / SPACING) * SPACING + origin,
      Math.ceil((value + REVEAL - origin) / SPACING) * SPACING + origin,
    ]
    const [firstX, lastX] = span(pointer.x)
    const [firstY, lastY] = span(pointer.y)
    context.fillStyle = getComputedStyle(this.fieldTarget).color
    for (let x = firstX; x <= lastX; x += SPACING) {
      for (let y = firstY; y <= lastY; y += SPACING) {
        const dx = x - pointer.x
        const dy = y - pointer.y
        const distance = Math.hypot(dx, dy)
        const scaled = distance / BULGE
        const lift = Math.exp(-Math.pow(scaled / 0.55, 2))
        const direction = distance ? (64 * scaled * lift) / distance : 0
        context.globalAlpha = 0.64 + lift * 0.36
        context.beginPath()
        context.arc(x + dx * direction, y + dy * direction - lift * 12, 1 + lift * 1.2, 0, Math.PI * 2)
        context.fill()
      }
    }
    context.restore()
    if (Math.abs(target.x - pointer.x) > 0.5 || Math.abs(target.y - pointer.y) > 0.5) this.schedule()
  }
}
