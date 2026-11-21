import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
async load() { try { this.data = await fetch(url); } catch (error) { console.error(error); } finally { this.loading = false; } }
}
