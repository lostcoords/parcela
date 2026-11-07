require "test_helper"

# The built site: every page, its metadata, links and downloads. Kiln's verifier already fails
# the build on a broken internal link, anchor or importmap module.
class SiteTest < ActiveSupport::TestCase
  include Kiln::TestHelper

  REPOSITORY = Rails.configuration.repository

  # One build for the whole class; it takes a few seconds.
  def self.output
    @output ||= begin
      dir = Dir.mktmpdir("parcela-site")
      build = Kiln::Build.new(output: dir)
      raise "build failed:\n#{build.report}" unless build.run.success?

      Minitest.after_run { FileUtils.rm_rf(dir) }
      Pathname(dir)
    end
  end

  def output = self.class.output
  def html(page) = output.join(page.path.delete_prefix("/"), "index.html").read
  def head(page) = Nokogiri::HTML5(html(page)).at_css("head")

  test "every Markdown file in docs/site is published" do
    sources = Docs::PAGES.map(&:source)
    REPOSITORY.glob("docs/site/*.md").each do |file|
      assert_includes sources, file.relative_path_from(REPOSITORY).to_s
    end
  end

  test "the sidebar starts with the docs index and has three guides" do
    assert_not Docs::GROUPS.flat_map(&:pages).include?(Docs::HOME)
    assert_equal 3, Docs::GROUPS.find { it.label == "Guides" }.pages.size
  end

  test "each page has one heading, the mascot and no local paths" do
    Docs::PAGES.each do |page|
      document = Nokogiri::HTML5(html(page))
      assert_equal 1, document.css("h1").size, "#{page.source} renders one h1"
      assert document.at_css('link[rel=icon][href*="logo"]'), "#{page.source} uses Fence Buddy"
      assert_not html(page).include?(Dir.home), "#{page.source} leaks no local path"
    end
  end

  test "each page has its own description, a canonical URL, a card and a sitemap entry" do
    sitemap = output.join("sitemap.xml").read
    descriptions = Docs::PAGES.map do |page|
      head = head(page)
      url = "https://parcela.run#{page.path}"
      meta = ->(name) { head.css(%(meta[name="#{name}"], meta[property="#{name}"])).map { it["content"] } }
      assert_equal 1, meta.("description").size, "#{page.source} has one description"
      assert_equal url, head.at_css("link[rel=canonical]")["href"]
      assert_equal [ url ], meta.("og:url")
      %w[og:title og:description twitter:card twitter:image].each { assert_equal 1, meta.(it).size, "#{page.source} has #{it}" }
      assert_equal [ "#171717" ], meta.("theme-color")
      assert_equal [ "https://parcela.run/og/#{page.slug}.png" ], meta.("og:image")
      assert_includes sitemap, "<loc>#{url}</loc>"
      meta.("description").first
    end
    assert_equal descriptions.uniq, descriptions, "descriptions are unique"
    cards = JSON.parse(output.join("cards.json").read).map { it["slug"] }
    assert_equal Docs::PAGES.map(&:slug), cards
    assert_includes output.join("robots.txt").read, "Sitemap: https://parcela.run/sitemap.xml"
  end

  test "downloads, the installer and the schema are served as is" do
    Docs::DOWNLOADS.each do |name, source|
      assert_equal REPOSITORY.join(source).binread, output.join("assets", name).binread, "download #{name}"
    end
    assert_equal Rails.root.join("install.sh").read, output.join("install.sh").read
    assert_equal Rails.root.join("schema/v1.json").read, output.join("schema/v1.json").read
  end

  test "search covers every searchable page" do
    documents = JSON.parse(output.join("search.json").read)["documents"]
    Docs.docs.select(&:searchable?).each do |page|
      assert documents.any? { it["url"] == page.path && it["title"] == page.meta[:title] }, "search lists #{page.source}"
    end
    assert documents.any? { it["url"] == "/docs/configuration/#hooks" }, "sections are searchable"
  end

  test "the tutorial contains the tested example app" do
    tutorial = REPOSITORY.join("docs/site/tutorial.md").read
    %w[index.html db.py api.py worker.py parcela.yaml].each do |name|
      example = REPOSITORY.join("examples/docs-demo", name).read.rstrip
      assert_includes tutorial, "\n#{example}\n```", "the tutorial contains #{name}"
    end
  end

  test "the rendered Claude skill install block matches the guide" do
    guide = REPOSITORY.join("docs/site/worktrees.md").read
    install = guide[/### Install the skill for Claude Code\n.*?```sh\n(.*?)\n```/m, 1]
    blocks = Nokogiri::HTML5(html(Docs.find("worktrees"))).css("pre").map { it.text.rstrip }
    assert_includes blocks, install
  end

  test "code blocks have copy buttons and diagrams keep their text" do
    page = Nokogiri::HTML5(html(Docs.find("how-it-works")))
    assert page.at_css("pre.diagram"), "diagrams render"
    assert_not page.at_css(".lc-code-block pre.diagram"), "diagrams have no copy button"
    assert Nokogiri::HTML5(html(Docs.find("configuration"))).at_css(".lc-code-block [data-action~='lc-copy#copy']")
  end
end
