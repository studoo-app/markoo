<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";

  let { value = $bindable(), id }: { value: string; id: string } = $props();

  async function browse() {
    const chosen = await open({ directory: true, multiple: false });
    if (typeof chosen === "string") value = chosen;
  }
</script>

<div class="row">
  <input {id} type="text" bind:value spellcheck="false" />
  <button type="button" onclick={browse}>Browse</button>
</div>
