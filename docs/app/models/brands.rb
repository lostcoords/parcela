# Tool marks from Simple Icons (CC0), listed in config/brands.json. Tool names in page prose gain
# their mark; code, headings and link text stay as written.
module Brands
  ALL = JSON.parse(Rails.root.join("config/brands.json").read, symbolize_names: true).freeze
  NAME = Regexp.new("\\b(#{ALL.map { Regexp.escape(it[:name]) }.sort_by { -it.length }.join("|")})\\b")
  UNMARKED = "a, code, pre, h1, h2, h3, h4, h5, h6".freeze

  def self.find(name) = ALL.find { it[:name] == name } or raise ArgumentError, "No brand #{name}"

  # Marks the tool names in a parsed fragment, in place.
  def self.mark!(fragment)
    fragment.xpath(".//text()").each do |text|
      next unless text.content.match?(NAME)
      next if text.ancestors.any? { it.element? && it.matches?(UNMARKED) }

      text.replace(text.content.split(NAME).each_with_index.map { |part, index| index.odd? ? mark(part) : ERB::Util.html_escape(part) }.join)
    end
    fragment
  end

  def self.mark(name)
    brand = find(name)
    %(<span class="tool-name" style="--tool-color:#{brand[:color]};--tool-dark:#{brand[:dark]}"><svg viewBox="0 0 24 24" aria-hidden="true" focusable="false"><path d="#{brand[:path]}"/></svg>#{ERB::Util.html_escape(name)}</span>)
  end
end
