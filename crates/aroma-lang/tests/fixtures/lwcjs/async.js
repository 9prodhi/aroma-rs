import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
async refresh() { this.rows = await fetch(this.url); }
}
