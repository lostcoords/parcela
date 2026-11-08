Rails.application.routes.draw do
  root "home#show"

  get "docs/", to: "pages#show", slug: "docs", as: :docs_index
  get "docs/:slug/", to: "pages#show", as: :page

  get "search", to: "search#show", format: :json
  # Titles and descriptions for the social cards, which `bin/cards` draws after the build.
  get "cards", to: "search#cards", format: :json
  get "robots", to: "search#robots", format: :text
  get "404", to: "pages#not_found"
  # Files served as downloads; see lib/downloads.rb.
  get "assets/:name", to: Downloads.new, constraints: { name: Regexp.union(Downloads::FILES.keys) }, format: false, as: :download
end
