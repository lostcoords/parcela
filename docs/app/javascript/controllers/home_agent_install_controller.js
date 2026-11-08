import { Controller } from "@hotwired/stimulus"

// A page cannot start a terminal agent, so the link copies the Get started prompt, which
// installs Parcela, saves its skill and configures the project, then opens it in the guide.
// Without clipboard access it still opens the prompt.
export default class extends Controller {
  static values = { prompt: String }

  async open(event) {
    event.preventDefault()
    try {
      await navigator.clipboard.writeText(this.promptValue)
    } catch {}
    window.location.href = this.element.href
  }
}
