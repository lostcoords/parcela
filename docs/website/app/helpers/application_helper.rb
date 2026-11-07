module ApplicationHelper
  RELEASE = "v0.8.0".freeze
  REPOSITORY = "https://github.com/lostcoords/parcela".freeze

  # The page's <head> metadata: description, canonical URL and the social card.
  def page_meta(page)
    meta = page.meta
    url = "#{Kiln.config.url}#{page.path}"
    image = "#{Kiln.config.url}#{page.card}"
    title = page_title(page)
    alt = "Parcela: #{meta[:title]}"
    safe_join([
      tag.meta(name: "description", content: meta[:description]),
      tag.link(rel: "canonical", href: url),
      tag.meta(property: "og:type", content: page == Docs::HOME ? "website" : "article"),
      tag.meta(property: "og:site_name", content: "Parcela"),
      tag.meta(property: "og:url", content: url),
      tag.meta(property: "og:title", content: title),
      tag.meta(property: "og:description", content: meta[:description]),
      tag.meta(property: "og:image", content: image),
      tag.meta(property: "og:image:type", content: "image/png"),
      tag.meta(property: "og:image:width", content: "1200"),
      tag.meta(property: "og:image:height", content: "630"),
      tag.meta(property: "og:image:alt", content: alt),
      tag.meta(name: "twitter:card", content: "summary_large_image"),
      tag.meta(name: "twitter:title", content: title),
      tag.meta(name: "twitter:description", content: meta[:description]),
      tag.meta(name: "twitter:image", content: image),
      tag.meta(name: "twitter:image:alt", content: alt)
    ], "\n")
  end

  def page_title(page)
    return "Parcela: #{page.front["title"]}" if page == Docs::HOME

    title = page.meta[:title]
    title.match?(/\bParcela\b/) ? title : "#{title} | Parcela"
  end

  # The page's HTML with each code block in lc_code_block, for its copy button.
  def page_html(page)
    fragment = Nokogiri::HTML5.fragment(page.html)
    fragment.css("pre.highlight").each_with_index do |pre, index|
      code = pre.at_css("code")
      block = lc_code_block(code.text, id: "code-#{index + 1}", language: pre["data-language"].presence, max_lines: nil, class: "highlight") { code.inner_html.html_safe }
      pre.replace(block.to_s)
    end
    fragment.to_html.html_safe
  end
end
