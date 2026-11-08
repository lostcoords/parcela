Kiln.configure do |config|
  config.output = Rails.root.join("../target/docs").to_s
  config.url = "https://parcela.run"
  config.not_found = "/404"
  # The installer and the schema live at the site root.
  config.files = { "install.sh" => "install.sh", "schema/v1.json" => "schema/v1.json" }
end

Kiln.pages do
  params :download, -> { Downloads::FILES.keys.map { { name: it } } }
  params :page, -> { Docs.docs.reject { it.slug == "docs" }.map { { slug: it.slug } } }
end
