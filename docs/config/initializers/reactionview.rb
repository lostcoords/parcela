# ReActionView renders ERB with Herb in development. Tensaku turns on ERB interception and adds
# Herb's source attribution itself; the options here are ReActionView's own.
# It's only loaded in development (it comes with Tensaku, in the development group).
if defined?(ReActionView)
  ReActionView.configure do |config|
    # Herb's dev tools (outlines, Open in Editor) are on by default in development. Tensaku
    # doesn't need them. Their script also waits for Herb's dev server, which only runs with
    # `slots`, and warns when nothing listens on its port.
    config.debug_mode = false

    # Herb's source attribution turns this on by default, which writes tag helpers out as static
    # markup and drops a double splat: `tag.button(**attributes)` loses every attribute, so
    # lostcoords-ui's search, code blocks and collapsibles stop working. Helpers then render as
    # usual, and Tensaku places their output by the nearest element written as markup.
    config.engine.parser_options = { action_view_helpers: false }
  end
end
