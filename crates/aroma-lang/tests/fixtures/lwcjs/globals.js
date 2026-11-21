import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
load() { window.setTimeout(() => console.log(JSON.stringify(Math.max(1, 2))), 1); return Promise.resolve(document.title); }
}
