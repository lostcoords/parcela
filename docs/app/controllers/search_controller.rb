class SearchController < ApplicationController
  def show
    render json: Docs::Search.json
  end

  def cards
    render json: Docs::PAGES.map { { slug: it.slug, **it.meta } }
  end

  def robots
    render plain: "User-agent: *\nAllow: /\n\nSitemap: #{Kiln.config.url}/sitemap.xml\n"
  end
end
