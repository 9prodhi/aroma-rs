import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
getValue(key) { return this[key] ?? records[key]; }
}
