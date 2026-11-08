require_relative "boot"

require "rails"
require "active_model/railtie"
require "action_controller/railtie"
require "action_view/railtie"

Bundler.require(*Rails.groups)

require "kiln/content"
require_relative "../lib/downloads"

module Site
  class Application < Rails::Application
    config.load_defaults Rails::VERSION::STRING.to_f
    config.autoload_lib(ignore: %w[assets tasks])
    config.middleware.insert_before 0, Downloads
  end
end
