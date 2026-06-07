local function as_number(value)
  if value == nil then
    return nil
  end
  return tonumber(value)
end

local function per_million_from_token_price(value)
  local num = as_number(value)
  if num == nil then
    return nil
  end
  return num * 1000000
end

local function pick_models(response)
  if response == nil then
    return nil
  end
  return response.data or response.models or response.items
end

local function supports_attachments_from(item)
  local architecture = item.architecture
  if architecture and architecture.input_modalities then
    local modalities = architecture.input_modalities
    for i = 1, #modalities do
      if modalities[i] == "image" or modalities[i] == "file" then
        return true
      end
    end
  end
  if item.supports_image_in == true or item.supports_vision == true then
    return true
  end
  return nil
end

local models = pick_models(model_response)
if models == nil then
  return {}
end

local patches = {}
for i = 1, #models do
  local item = models[i]
  local pricing = item.pricing or {}
  patches[#patches + 1] = {
    id = item.id,
    name = item.name,
    context_window = item.context_length,
    max_output_tokens = item.top_provider and item.top_provider.max_completion_tokens or nil,
    reasoning = item.supports_reasoning,
    vision = item.supports_image_in or item.supports_vision,
    supports_attachments = supports_attachments_from(item),
    input_cost_per_million = per_million_from_token_price(pricing.prompt),
    output_cost_per_million = per_million_from_token_price(pricing.completion),
    cached_input_cost_per_million = per_million_from_token_price(pricing.cached_prompt),
    cached_output_cost_per_million = per_million_from_token_price(pricing.cached_completion),
  }
end

return patches
