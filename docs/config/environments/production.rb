# Only used by `bin/rails kiln:build`: the site is static, nothing runs in production.
Rails.application.configure do
  config.enable_reloading = false
  config.eager_load = true
  config.consider_all_requests_local = false
  config.action_controller.perform_caching = true
  config.public_file_server.enabled = true
  config.secret_key_base = "site-static-build"
  config.log_level = :warn
  config.hosts.clear
end
