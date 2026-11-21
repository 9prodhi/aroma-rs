import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
describe() { return `Hello ${this.name}, ${count} records`; }
}
