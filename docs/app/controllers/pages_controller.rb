class PagesController < ApplicationController
  def show
    @page = Docs.find(params[:slug]) or raise ActionController::RoutingError, "No page #{params[:slug]}"
    raise ActionController::RoutingError, "No page #{params[:slug]}" if @page == Docs::HOME

    @html = @page.file ? Kiln::Content.file(@page.file).html : render_to_string(template: "pages/#{@page.slug}", layout: false)
  end

  def not_found
    render status: :not_found
  end
end
