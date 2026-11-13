# Writes the dashboard's pages from lostcoords-ui components. Runs through docs/, the only Rails app
# here; the Parcela binary serves the results and needs no Ruby. scripts/build-dashboard runs it.
root = File.expand_path("../..", __dir__)
output = ENV.fetch("DASHBOARD_OUT", "#{root}/src/proxy/dashboard")
{ "page" => [ "index.html", "app.js" ], "logs" => [ "logs.html", "logs.js" ] }.each do |source, (file, script)|
  body = ApplicationController.render(inline: File.read("#{root}/scripts/dashboard/#{source}.html.erb"), layout: false)
  File.write("#{output}/#{file}", <<~HTML)
    <!doctype html>
    <html lang="en" data-theme="dark">
    <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Parcela</title>
    <link rel="icon" href="/icon.png">
    <link rel="stylesheet" href="/ui.css">
    <script src="/#{script}" defer></script>
    </head>
    <body class="bg-background text-foreground">
    #{body.strip}
    </body>
    </html>
  HTML
end
