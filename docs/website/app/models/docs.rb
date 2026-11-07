# The pages of parcela.run and where their Markdown lives in the repository. Each page renders
# its source file; relative links between sources become public routes, so the same Markdown
# reads well on GitHub and on the site.
module Docs
  class UnpublishedLink < StandardError; end

  Page = Data.define(:slug, :title, :source) do
    def path = Docs.route(slug)
    def file = Rails.configuration.repository.join(source)
    def markdown = Docs.markdown(self)
    def front = Docs.frontmatter(file.read)
    def html = Docs.render(self)
    def meta = Docs.describe(self)
    def card = "/og/#{slug}.png"
    def searchable? = front["search"] != false
  end

  Group = Data.define(:label, :pages)

  GROUPS = [
    Group.new("Start here", [
      Page.new("docs", "Documentation", "docs/site/docs.md"),
      Page.new("getting-started", "Get started", "docs/site/getting-started.md")
    ]),
    Group.new("Guides", [
      Page.new("frameworks", "Configure your app", "docs/site/frameworks.md"),
      Page.new("worktrees", "Worktrees and agents", "docs/site/worktrees.md"),
      Page.new("troubleshooting", "Troubleshooting", "docs/site/troubleshooting.md")
    ]),
    Group.new("Reference", [
      Page.new("cli", "CLI reference", "docs/site/cli.md"),
      Page.new("configuration", "Configuration", "docs/site/configuration.md"),
      Page.new("how-it-works", "How it works", "docs/site/how-it-works.md"),
      Page.new("faq", "FAQ", "docs/site/faq.md"),
      Page.new("telemetry", "Telemetry", "docs/site/telemetry.md")
    ]),
    Group.new("More", [
      Page.new("tutorial", "Example app", "docs/site/tutorial.md"),
      Page.new("contributing", "Contribute to Parcela", "CONTRIBUTING.md")
    ])
  ].freeze

  HOME = Page.new("index", "Home", "docs/site/index.md")

  # Every page: the landing page, the sidebar, and pages linked from the docs but not listed.
  PAGES = [
    HOME,
    *GROUPS.flat_map(&:pages),
    Page.new("operating-skill", "Operating skill", ".agents/skills/parcela/SKILL.md"),
    Page.new("architecture", "Architecture", "docs/architecture.md"),
    Page.new("docs-site", "Build the documentation site", "docs/website/README.md")
  ].freeze

  DOWNLOADS = Downloads::FILES

  FRONTMATTER = /\A---\n(.*?)\n---\n/m

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

    def frontmatter(text)
      (match = FRONTMATTER.match(text)) ? YAML.safe_load(match[1]) || {} : {}
    end

    # The page's Markdown with relative links rewritten to routes. Includes and scripts are
    # refused, as on the old site, so nothing in a source runs on parcela.run.
    def markdown(page)
      text = page.file.read
      raise ArgumentError, "Includes and scripts are not allowed: #{page.source}" if text.match?(/<!--\s*@include:|<script/i)

      text = text.sub(FRONTMATTER, "")
      text.gsub(/\]\(([^)\s]+)\)/) do
        reference = $1
        next "](#{reference})" if reference.match?(%r{\A(?:[a-z]+:|#|/)}i)

        file, suffix = reference.split(/(?=[?#])/, 2)
        target = page.file.dirname.join(file).expand_path
        link = suffix == "?raw=1" ? raws[target] : routes[target]&.then { "#{it}#{suffix}" }
        raise UnpublishedLink, "Link to an unpublished file: #{reference} from #{page.source}" unless link

        "](#{link})"
      end
    end

    def render(page) = cache(page, :html) { Markdown.render(markdown(page)) }

    # Headings for "On this page": [level, id, text] for each h2 and h3.
    def outline(page)
      cache(page, :outline) do
        Nokogiri::HTML5.fragment(render(page)).css("h2[id], h3[id]").map { [ it.name[1].to_i, it["id"], it.text.squish ] }
      end
    end

    # Title and description for search results, the <head> and the social card: the
    # frontmatter's when it has them, else the first heading (the sidebar name when the heading
    # is just "Parcela") and the first paragraph that does not lead into a list or code block.
    def describe(page)
      cache(page, :meta) do
        text = page.file.read
        front = frontmatter(text)
        body = text.sub(FRONTMATTER, "")
        heading = plain(body[/^# (.+)$/, 1].to_s)
        title = front["title"] || (heading == "Parcela" ? page.title : heading)
        paragraph = body.split(/\n{2,}/).map(&:strip).find { it.match?(/\A[A-Za-z`*\[]/) && !it.start_with?("```") && !it.end_with?(":") }
        description = front["description"] || plain(paragraph.to_s)
        if description.length > 180
          cut = description[0, 180]
          description = cut.include?(". ") ? cut[0..cut.rindex(". ")] : "#{cut.sub(/\s+\S*\z/, "")}…"
        end
        { title:, description: }
      end
    end

    private
      def plain(text)
        text.gsub(/!?\[([^\]]*)\]\([^)]*\)/, '\1').gsub(/`([^`]*)`/, '\1').gsub(/\*\*?([^*]+)\*\*?/, '\1').squish
      end

      def routes = @routes ||= PAGES.to_h { [ it.file.expand_path, it.path ] }
      def raws = @raws ||= DOWNLOADS.to_h { |name, source| [ Rails.configuration.repository.join(source).expand_path, "/assets/#{name}" ] }

      # Rendered once per build; in development, again whenever the source changes.
      def cache(page, kind)
        @cache ||= {}
        key = [ page.slug, kind ]
        mtime = page.file.mtime
        entry = @cache[key]
        return entry[1] if entry && entry[0] == mtime

        yield.tap { @cache[key] = [ mtime, it ] }
      end
  end
end
