import { mount } from 'svelte'
import App from './App.svelte'
import '@my-tauri-plugins/plugin-speech'

const app = mount(App, {
  target: document.getElementById('app')!,
})

export default app
