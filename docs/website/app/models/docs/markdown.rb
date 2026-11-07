# Markdown to HTML for the site's pages: GitHub Flavored Markdown through Kiln's commonmarker
# settings, with three differences from a Kiln .md view.
#
# - Raw HTML in a source renders as text. The sources are also read on GitHub and by agents.
# - ```diagram fences stay plain text that keeps its columns (see .diagram in site.css).
# - Tool names in prose (Rails, Claude Code, ...) gain their mark; code and link text stay as written.
#
# Code blocks come out as <pre data-language> with Rouge's classes inside; the view wraps each
# one in lc_code_block so it gets a copy button.
module Docs
  module Markdown
    OPTIONS = Kiln::Content::Markdown::OPTIONS
    BRANDS = JSON.parse(Rails.root.join("config/brands.json").read, symbolize_names: true).freeze
    BRAND = Regexp.new("\\b(#{BRANDS.map { Regexp.escape(it[:name]) }.sort_by { -it.length }.join("|")})\\b")
    NO_BRANDS = "a, code, pre, h1, h2, h3, h4, h5, h6".freeze

    def self.render(text)
      document = Commonmarker.parse(text.to_s, options: OPTIONS)
      nodes = []
      document.walk { nodes << it if %i[html_block html_inline code_block].include?(it.type) }
      nodes.each { replace(it) }
      html = document.to_html(options: OPTIONS, plugins: Kiln::Content::Markdown::PLUGINS)
      brand(html.gsub(Kiln::Content::Markdown::HEADING_ANCHOR, ""))
    end

    RAW = { render: { unsafe: true } }.freeze

    def self.replace(node)
      case node.type
      when :html_inline
        node.replace(Commonmarker::Node.new(:text, content: node.to_html(options: RAW)))
      when :html_block
        paragraph = Commonmarker::Node.new(:paragraph)
        paragraph.append_child(Commonmarker::Node.new(:text, content: node.to_html(options: RAW).chomp))
        node.replace(paragraph)
      when :code_block
        language = node.fence_info.split.first.to_s
        code = node.string_content
        html = if language == "diagram"
          %(<pre class="diagram">#{ERB::Util.html_escape(code)}</pre>\n)
        else
          lexer = Rouge::Lexer.find_fancy(language, code) || Rouge::Lexers::PlainText.new
          %(<pre class="highlight" data-language="#{ERB::Util.html_escape(language)}"><code>#{Rouge::Formatters::HTML.new.format(lexer.lex(code))}</code></pre>\n)
        end
        node.replace(Commonmarker::Node.new(:html_block, literal: html))
      end
    end

    def self.brand(html)
      return html unless html.match?(BRAND)

      fragment = Nokogiri::HTML5.fragment(html)
      fragment.xpath(".//text()").each do |text|
        next unless text.content.match?(BRAND)
        next if text.ancestors.any? { it.respond_to?(:matches?) && it.element? && it.matches?(NO_BRANDS) }

        text.replace(text.content.split(BRAND).each_with_index.map { |part, index| index.odd? ? mark(part) : ERB::Util.html_escape(part) }.join)
      end
      fragment.to_html
    end

    def self.mark(name)
      brand = BRANDS.find { it[:name] == name }
      %(<span class="tool-name" style="--tool-color:#{brand[:color]};--tool-dark:#{brand[:dark]}"><svg viewBox="0 0 24 24" aria-hidden="true" focusable="false"><path d="#{brand[:path]}"/></svg>#{ERB::Util.html_escape(name)}</span>)
    end
  end
end
