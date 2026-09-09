import js from '@eslint/js'
import pluginVue from 'eslint-plugin-vue'
import globals from 'globals'

export default [
  { ignores: ['dist/**', 'node_modules/**', 'src-tauri/**'] },
  js.configs.recommended,
  ...pluginVue.configs['flat/essential'],
  {
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: 'module',
      globals: { ...globals.browser, ...globals.node },
    },
    rules: {
      // Advisory for now; tightened to 'error' once existing warnings are cleared.
      'no-unused-vars': 'warn',
      'vue/multi-word-component-names': 'off',
      'vue/use-v-on-exact': 'warn',
    },
  },
]
