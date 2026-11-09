# Writes src/dashboard/index.html from lostcoords-ui components. Runs through docs/, the only Rails
# app here; the Parcela binary serves the result and needs no Ruby. scripts/build-dashboard runs it.
root = File.expand_path("../..", __dir__)
body = ApplicationController.render(inline: File.read("#{root}/scripts/dashboard/page.html.erb"), layout: false)
html = <<~HTML
  <!doctype html>
  <html lang="en" data-theme="dark">
  <head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Parcela</title>
  <link rel="icon" href="/_parcela/badge.png">
  <link rel="stylesheet" href="/ui.css">
  <script src="/app.js" defer></script>
  </head>
  <body class="bg-background text-foreground">
  #{body.strip}
  </body>
  </html>
HTML
File.write(ENV.fetch("DASHBOARD_HTML", "#{root}/src/dashboard/index.html"), html)
