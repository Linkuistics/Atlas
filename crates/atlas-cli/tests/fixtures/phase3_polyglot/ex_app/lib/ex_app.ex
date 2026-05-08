defprotocol Stringable do
  @doc "Convert a value to a string representation."
  @callback to_string(t) :: String.t()
end

defmodule ExApp do
  @moduledoc "Public Elixir module with one public function."

  def hello, do: "hello from elixir"
end
