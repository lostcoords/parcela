class PagesController < ApplicationController
  def show
    @page = Docs.find(params[:slug]) or raise ActionController::RoutingError, "No page #{params[:slug]}"
    raise ActionController::RoutingError, "No page #{params[:slug]}" if @page == Docs::HOME
  end

  def not_found
    render status: :not_found
  end
end
