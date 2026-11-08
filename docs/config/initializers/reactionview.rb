# ReActionView renders ERB with Herb in development. Tensaku turns on ERB interception and adds
# Herb's source attribution itself; the options here are ReActionView's own.
# It's only loaded in development (it comes with Tensaku, in the development group).
if defined?(ReActionView)
  ReActionView.configure do |config|
    # Herb's dev tools (outlines, Open in Editor) are on by default in development. Tensaku
    # doesn't need them. Their script also waits for Herb's dev server, which only runs with
    # `slots`, and warns when nothing listens on its port.
    config.debug_mode = false
  end
end
