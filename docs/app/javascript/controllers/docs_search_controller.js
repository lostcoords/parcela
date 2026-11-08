import { Controller } from "@hotwired/stimulus"
import MiniSearch from "minisearch"

const PER_GROUP = 6
const SEARCH = {
  boost: { title: 4, keywords: 2, detail: 1.5 },
  combineWith: "AND",
  // The last word may still be half typed; longer words forgive a typo or two.
  prefix: (_term, index, terms) => index === terms.length - 1,
  fuzzy: (term) => (term.length > 3 ? 0.2 : false),
}

// Answers lc-search:query for the docs: /search.json downloads and MiniSearch indexes it while
// the page is idle, and each query renders its best matches, grouped, from the templates.
export default class extends Controller {
  static targets = ["group", "result"]
  static values = { url: String }

  connect() {
    this.idle = (window.requestIdleCallback ?? setTimeout)(() => this.prepare().catch(() => {}))
  }

  disconnect() {
    ;(window.cancelIdleCallback ?? clearTimeout)(this.idle)
  }

  query(event) {
    event.preventDefault()
    this.latest = event.detail
    if (this.index) return this.render()
    this.prepare()
      .then(() => this.render())
      .catch((error) => console.error(error))
  }

  prepare() {
    this.preparing ??= fetch(this.urlValue)
      .then((response) => {
        if (!response.ok) throw new Error(`${this.urlValue} answered ${response.status}`)
        return response.json()
      })
      .then(({ icons, documents }) => {
        this.groups = Object.keys(icons)
        this.documents = documents.map((document, id) => ({ ...document, id }))
        this.index = new MiniSearch({ fields: ["title", "keywords", "detail", "text"] })
        this.index.addAll(this.documents)
      })
      .catch((error) => {
        this.preparing = null
        throw error
      })
    return this.preparing
  }

  render() {
    const { query, results } = this.latest
    const matches = query
      ? this.index
          .search(query, {
            ...SEARCH,
            boostDocument: (id) => titleBoost(this.documents[id].title, query),
          })
          .map((match) => ({ match, ...this.documents[match.id] }))
      : this.documents.filter((entry) => entry.featured)
    // Matches arrive best first, so each group appears where its best match ranks.
    const names = query ? [...new Set(matches.map((entry) => entry.group))] : this.groups
    results.replaceChildren(
      ...names.flatMap((name) => {
        const entries = matches.filter((entry) => entry.group === name).slice(0, PER_GROUP)
        return entries.length ? [this.renderGroup(name, entries)] : []
      }),
    )
  }

  renderGroup(name, entries) {
    const group = this.groupTarget.content.firstElementChild.cloneNode(true)
    group.setAttribute("aria-label", name)
    group.querySelector("[data-slot=search-group-title]").textContent = name
    group.append(...entries.map((entry) => this.renderResult(entry)))
    return group
  }

  // A result found by its text shows where the words appear; otherwise its one-line description.
  renderResult(entry) {
    const template = this.resultTargets.find((target) => target.dataset.group === entry.group)
    const result = template.content.firstElementChild.cloneNode(true)
    result.href = entry.url
    result.querySelector("[data-slot=search-result-title]").textContent = entry.title
    const detail = result.querySelector("[data-slot=search-result-detail]")
    const fields = Object.values(entry.match?.match ?? {}).flat()
    if (fields.includes("text") && !fields.includes("title")) {
      detail.replaceChildren(...excerpt(entry.text, entry.match.terms))
    } else {
      detail.textContent = entry.detail
    }
    return result
  }
}

// A title that is the query, or starts with it, outranks pages that only mention it.
function titleBoost(title, query) {
  const [name, words] = [title, query].map((text) =>
    text.toLowerCase().replace(/[_-]+/g, " ").trim(),
  )
  if (name === words) return 4
  return name.startsWith(words) ? 2 : 1
}

// About a line of text around the matched words, with every one of them marked.
function excerpt(text, terms) {
  const pattern = new RegExp(`(${terms.map(escapeRegExp).join("|")})`, "giu")
  // Prefer the spot where the words appear together, as typed.
  const phrase = new RegExp(terms.map(escapeRegExp).join("\\W+"), "iu")
  const first = terms.length > 1 && phrase.test(text) ? text.search(phrase) : text.search(pattern)
  const from = first > 50 ? text.lastIndexOf(" ", first - 40) + 1 : 0
  const to = text.indexOf(" ", first + 120)
  const slice = `${from > 0 ? "…" : ""}${text.slice(from, to === -1 ? undefined : to)}${to === -1 ? "" : "…"}`
  return slice.split(pattern).map((part, index) => {
    if (index % 2 === 0) return part
    const mark = document.createElement("mark")
    mark.textContent = part
    return mark
  })
}

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}
