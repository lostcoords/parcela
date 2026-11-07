require "test_helper"

class MarkdownTest < ActiveSupport::TestCase
  def render(text) = Docs::Markdown.render(text)

  test "tool names are marked without matching lookalikes" do
    html = render("Claude Code, Codex, Rails, and Next.js. NextXjs and GitHub.")
    assert_equal 4, html.scan('class="tool-name"').size
    assert_includes html, 'aria-hidden="true"'
  end

  test "code and link text stay as written" do
    assert_not_includes render("`Rails`"), "tool-name"
    assert_not_includes render("```sh\nCodex Rails\n```"), "tool-name"
    linked = render("Python and [Python with **Caddy** installed](https://example.com) and Caddy.")
    assert_equal 2, linked.scan('class="tool-name"').size
    assert_no_match %r{<a\b[^>]*>.*?tool-name.*?</a>}m, linked
  end

  test "raw HTML renders as text" do
    assert_includes render("<script>alert(1)</script>"), "&lt;script&gt;"
    assert_includes render("Inline <b>bold</b>."), "&lt;b&gt;"
  end

  test "diagram fences keep their columns" do
    html = render("```diagram\n a ── b\n │\n```\n")
    assert_includes html, %(<pre class="diagram"> a ── b\n │\n</pre>)
  end

  test "relative links become routes and unpublished links fail" do
    assert_includes Docs.find("docs").markdown, "](/docs/getting-started/)"
    file = Pathname(Dir.mktmpdir).join("x.md").tap { it.write("[a](missing.md)") }
    assert_raises(Docs::UnpublishedLink) { Docs.markdown(Struct.new(:file, :source).new(file, "x.md")) }
  end
end
