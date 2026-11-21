import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
build(items) { return new Map(items.map(item => [item.Id, item])); }
}
