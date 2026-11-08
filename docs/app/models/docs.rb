# The pages of parcela.run. Most are Kiln Markdown views in app/views/pages; a few are Markdown
# files that live elsewhere in the repository (CONTRIBUTING.md, the operating skill, this site's
# README) and render as Kiln content entries.
module Docs
  REPOSITORY = Pathname(__dir__).join("../../..").expand_path

  Page = Data.define(:slug, :title, :description, :file) do
    def initialize(slug:, title:, description:, file: nil) = super
    def path = Docs.route(slug)
    def card = "/og/#{slug}.png"
    def html = Docs.html(self)
    def meta = { title:, description: }
  end

  Group = Data.define(:label, :pages)

  GROUPS = [
    Group.new("Start here", [
      Page.new("docs", "Documentation", "Run each checkout at its own HTTPS URL. Start with the task you need."),
      Page.new("getting-started", "Get started", "Start your existing app at a named HTTPS URL, with a coding agent or by hand.")
    ]),
    Group.new("Guides", [
      Page.new("frameworks", "Configure your app", "Put the commands your app already uses in parcela.yaml. Configure data once so every checkout can start with parcela up."),
      Page.new("worktrees", "Worktrees and agents", "Start with a configured project. Give each agent its own checkout, then run the same parcela.yaml in each one."),
      Page.new("troubleshooting", "Troubleshooting", "What to check when an environment does not start, a URL does not load or the browser does not trust the certificate, starting with parcela doctor and parcela status --current.")
    ]),
    Group.new("Reference", [
      Page.new("cli", "CLI reference", "parcela [GLOBAL OPTIONS] COMMAND acts on the current checkout unless another target is selected. With no command, it shows checkout status or the next setup step."),
      Page.new("configuration", "Configuration", "Parcela reads parcela.yaml from the root of your checkout. Commands look for it in the current directory, or --path, and then in each parent directory."),
      Page.new("how-it-works", "How it works", "Parcela is a CLI, one supervisor process per running environment, and one shared proxy. Caddy does TLS and routing; Parcela tells it what to route."),
      Page.new("faq", "FAQ", "Parcela stores its registry, logs, certificates, and environment data in a private state directory. The default is ~/.local/state/parcela."),
      Page.new("telemetry", "Telemetry", "What Parcela's anonymous usage data contains, where it goes, and how to see or turn it off.")
    ]),
    Group.new("More", [
      Page.new("tutorial", "Example app", "Build a page, a SQLite-backed API, and a worker. Then run a second copy in a worktree with its own database."),
      Page.new("contributing", "Contribute to Parcela", "Parcela's Rust CLI supervises local development processes. Caddy handles HTTPS and routing. Read Architecture before changing lifecycle, routing, state, or process ownership.", REPOSITORY.join("CONTRIBUTING.md"))
    ])
  ].freeze

  HOME = Page.new("index", "Isolated dev environments for your AI agents", "Run your app in every worktree, with separate processes, free ports, and named HTTPS URLs. Local development for parallel coding agents on macOS and Linux.")

  # Every page: the landing page, the sidebar, and pages linked from the docs but not listed.
  PAGES = [
    HOME,
    *GROUPS.flat_map(&:pages),
    Page.new("operating-skill", "Operating skill", "The instructions parcela --skill prints for a coding agent: how to pick a checkout, start and stop it, read URLs and logs and share it.", REPOSITORY.join(".agents/skills/parcela/SKILL.md")),
    Page.new("architecture", "Architecture", "Parcela turns one checkout's configuration into a supervised environment. It owns identity, process lifecycle, and reservations."),
    Page.new("docs-site", "Build the documentation site", "https://parcela.run is a small Rails app that Kiln renders to static files, styled with lostcoords-ui. The pages come from Kiln Markdown views.", Rails.root.join("README.md"))
  ].freeze

  class << self
    def find(slug) = PAGES.find { it.slug == slug }
    def docs = PAGES.reject { it == HOME }

    def route(slug)
      case slug
      when "index" then "/"
      when "docs" then "/docs/"
      else "/docs/#{slug}/"
      end
    end

    # The page's body: its Markdown view, or its file as a Kiln content entry.
    def html(page)
      page.file ? Kiln::Content.file(page.file).html : ApplicationController.render(template: "pages/#{page.slug}", layout: false)
    end

    # Headings for "On this page": [level, id, text] for each h2 and h3.
    def outline(html)
      Nokogiri::HTML5.fragment(html).css("h2[id], h3[id]").map { [ it.name[1].to_i, it["id"], it.text.squish ] }
    end
  end
end
