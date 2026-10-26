const stub = new URL("./cf-stub.mjs", import.meta.url).href;

export async function resolve(specifier, context, nextResolve) {
  if (specifier === "cloudflare:workers") {
    return { url: stub, shortCircuit: true };
  }
  return nextResolve(specifier, context);
}
