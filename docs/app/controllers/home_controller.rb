class HomeController < ApplicationController
  def show
    @page = Docs::HOME
  end
end
