# The documents behind the search dialog, served as /search.json. MiniSearch indexes them in
# the browser, so search works the same on the dev server and the static site. Each page is one
# document, and so is each of its h2 and h3 sections, with the text up to the next heading.
module Docs
  module Search
    GROUPS = { "Pages" => "document-text", "Sections" => "hashtag" }.freeze
    SKIPPED = "pre, svg, script, template".freeze

    Document = Data.define(:group, :title, :detail, :url, :text, :featured)

    def self.json
      JSON.generate(icons: GROUPS, documents: documents.map(&:to_h)) + "\n"
    end

    def self.documents
      pages = Docs.docs
      pages.map { page_document(it) } + pages.flat_map { section_documents(it) }
    end

    def self.page_document(page)
      meta = page.meta
      body = Nokogiri::HTML5.fragment(page.html)
      body.css(SKIPPED).each(&:remove)
      featured = Docs::GROUPS.first(2).flat_map(&:pages).include?(page)
      Document.new(group: "Pages", title: meta[:title], detail: meta[:description], url: page.path, text: body.css("p, li").map(&:text).join(" ").squish, featured:)
    end

    def self.section_documents(page)
      sections = []
      Nokogiri::HTML5.fragment(page.html).children.each do |node|
        if node.element? && %w[h2 h3].include?(node.name) && node["id"]
          sections << { title: node.text.squish, url: "#{page.path}##{node["id"]}", text: +"" }
        elsif sections.any? && node.element? && !node.matches?(SKIPPED)
          sections.last[:text] << " " << node.text
        end
      end
      sections.map { Document.new(group: "Sections", title: it[:title], detail: page.meta[:title], url: it[:url], text: it[:text].squish, featured: false) }
    end
  end
end
